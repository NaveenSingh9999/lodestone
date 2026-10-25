//! Remote management API (spec 4.13): optional HTTP + WebSocket surface
//! exposing the same commands and event stream as section 3.3.
//!
//! Off by default. Auth is a bearer token (a PIN is just a short token)
//! with attempt lockout. The core refuses non-loopback binds unless the
//! caller explicitly allows WAN exposure, and the FFI layer must never
//! route this port through the public tunnel.

use std::net::SocketAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;

use crate::events::Event;

const WS_GUID: &[u8] = b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
const MAX_HEADER_BYTES: usize = 16 * 1024;
const MAX_BODY_BYTES: usize = 1024 * 1024;
const MAX_WS_FRAME: usize = 64 * 1024;

#[derive(Debug, Clone)]
pub struct RemoteApiConfig {
    pub bind: SocketAddr,
    pub token: String,
    pub allow_wan: bool,
    pub max_attempts: u32,
    pub lockout: Duration,
}

impl Default for RemoteApiConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:7300".parse().expect("valid loopback bind"),
            token: String::new(),
            allow_wan: false,
            max_attempts: 5,
            lockout: Duration::from_secs(300),
        }
    }
}

/// The core-side command surface shared with the FFI: JSON in, JSON out.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiRequest {
    pub command: String,
    #[serde(default)]
    pub args: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiResponse {
    pub ok: bool,
    #[serde(default)]
    pub data: serde_json::Value,
}

pub trait RemoteBackend: Send + Sync {
    fn dispatch(&self, req: ApiRequest) -> ApiResponse;
    fn subscribe(&self) -> tokio::sync::broadcast::Receiver<Event>;
}

#[derive(Debug, PartialEq, Eq)]
pub enum AuthError {
    Unauthorized,
    Locked(Duration),
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Token auth with attempt lockout (spec 4.13).
pub struct AuthGate {
    token: Vec<u8>,
    max_attempts: u32,
    lockout: Duration,
    state: Mutex<GateState>,
}

#[derive(Default)]
struct GateState {
    fails: u32,
    locked_until: Option<Instant>,
}

impl AuthGate {
    pub fn new(token: String, max_attempts: u32, lockout: Duration) -> Self {
        Self {
            token: token.into_bytes(),
            max_attempts,
            lockout,
            state: Mutex::new(GateState::default()),
        }
    }

    pub fn check(&self, provided: Option<&str>) -> Result<(), AuthError> {
        let mut st = self.state.lock().unwrap();
        if let Some(until) = st.locked_until {
            let now = Instant::now();
            if now < until {
                return Err(AuthError::Locked(until - now));
            }
            st.locked_until = None;
        }
        match provided.filter(|t| constant_time_eq(t.as_bytes(), &self.token)) {
            Some(_) => {
                st.fails = 0;
                Ok(())
            }
            None => {
                st.fails += 1;
                if st.fails >= self.max_attempts {
                    st.locked_until = Some(Instant::now() + self.lockout);
                    return Err(AuthError::Locked(self.lockout));
                }
                Err(AuthError::Unauthorized)
            }
        }
    }
}

pub fn ensure_bind_allowed(bind: SocketAddr, allow_wan: bool) -> Result<(), &'static str> {
    if !bind.ip().is_loopback() && !allow_wan {
        Err("non-loopback bind requires allow_wan (spec 4.13)")
    } else {
        Ok(())
    }
}

pub struct ParsedRequest {
    pub method: String,
    pub path: String,
    pub query: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl ParsedRequest {
    pub fn header(&self, name: &str) -> Option<&str> {
        let lower = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(k, _)| *k == lower)
            .map(|(_, v)| v.as_str())
    }

    pub fn query_param(&self, key: &str) -> Option<String> {
        for pair in self.query.split('&') {
            let mut it = pair.splitn(2, '=');
            if it.next()? == key {
                return it.next().map(|v| v.to_string());
            }
        }
        None
    }
}

pub struct HttpResponse {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn json(status: u16, value: &serde_json::Value) -> Self {
        Self {
            status,
            content_type: "application/json",
            body: value.to_string().into_bytes(),
        }
    }
}

fn status_text(code: u16) -> &'static str {
    match code {
        101 => "Switching Protocols",
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        429 => "Too Many Requests",
        _ => "Internal Server Error",
    }
}

pub fn encode_response(resp: &HttpResponse) -> Vec<u8> {
    let mut out = format!(
        "HTTP/1.1 {} {}\r\ncontent-type: {}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        resp.status,
        status_text(resp.status),
        resp.content_type,
        resp.body.len()
    )
    .into_bytes();
    out.extend_from_slice(&resp.body);
    out
}

pub fn parse_request(raw: &[u8]) -> Option<ParsedRequest> {
    let head_end = raw.windows(4).position(|w| w == b"\r\n\r\n")? + 4;
    let head = std::str::from_utf8(&raw[..head_end]).ok()?;
    let mut lines = head.split("\r\n");
    let request_line = lines.next()?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next()?.to_string();
    let target = parts.next()?.to_string();
    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p.to_string(), q.to_string()),
        None => (target, String::new()),
    };
    let mut headers = Vec::new();
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            headers.push((k.to_ascii_lowercase(), v.trim().to_string()));
        }
    }
    let content_length: usize = headers
        .iter()
        .find(|(k, _)| k == "content-length")
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(0);
    if raw.len() < head_end + content_length {
        return None;
    }
    let body = raw[head_end..head_end + content_length].to_vec();
    Some(ParsedRequest {
        method,
        path,
        query,
        headers,
        body,
    })
}

pub fn base64_encode(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = chunk.get(1).map(|v| u32::from(*v)).unwrap_or(0);
        let b2 = chunk.get(2).map(|v| u32::from(*v)).unwrap_or(0);
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

fn ws_accept(key: &str) -> String {
    let mut h = Sha1::new();
    h.update(key.as_bytes());
    h.update(WS_GUID);
    base64_encode(&h.finalize())
}

pub struct RemoteApi {
    pub cfg: RemoteApiConfig,
    pub backend: Box<dyn RemoteBackend>,
    gate: AuthGate,
}

impl RemoteApi {
    pub fn new(cfg: RemoteApiConfig, backend: Box<dyn RemoteBackend>) -> Self {
        if ensure_bind_allowed(cfg.bind, cfg.allow_wan).is_err() {
            panic!("remote api: non-loopback bind requires allow_wan (spec 4.13)");
        }
        let gate = AuthGate::new(cfg.token.clone(), cfg.max_attempts, cfg.lockout);
        Self { cfg, backend, gate }
    }

    pub async fn run(self) -> std::io::Result<()> {
        let listener = TcpListener::bind(self.cfg.bind).await?;
        self.serve(listener).await
    }

    /// Serves on an already-bound listener (tests bind an ephemeral port).
    pub async fn serve(self, listener: TcpListener) -> std::io::Result<()> {
        let api = Arc::new(self);
        loop {
            let (stream, _) = listener.accept().await?;
            let api = Arc::clone(&api);
            tokio::spawn(async move {
                handle_conn(stream, &api).await;
            });
        }
    }
}

async fn handle_conn(mut stream: TcpStream, api: &RemoteApi) {
    let mut raw = Vec::new();
    let mut buf = [0u8; 2048];
    loop {
        match stream.read(&mut buf).await {
            Ok(0) | Err(_) => return,
            Ok(n) => {
                raw.extend_from_slice(&buf[..n]);
                if let Some(req) = parse_request(&raw) {
                    route(stream, req, api).await;
                    return;
                }
                if raw.len() > MAX_HEADER_BYTES + MAX_BODY_BYTES {
                    return;
                }
            }
        }
    }
}

async fn route(mut stream: TcpStream, req: ParsedRequest, api: &RemoteApi) {
    if req.path == "/health" {
        let resp = HttpResponse::json(200, &serde_json::json!({"ok": true}));
        let _ = stream.write_all(&encode_response(&resp)).await;
        return;
    }
    let presented = req
        .header("authorization")
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_string)
        .or_else(|| req.query_param("token"));
    match api.gate.check(presented.as_deref()) {
        Ok(()) => {}
        Err(err) => {
            let status = match err {
                AuthError::Locked(_) => 429,
                AuthError::Unauthorized => 401,
            };
            let body = serde_json::json!({"ok": false, "error": "auth failed"});
            let _ = stream
                .write_all(&encode_response(&HttpResponse::json(status, &body)))
                .await;
            return;
        }
    }
    let is_ws = req
        .header("upgrade")
        .map(|v| v.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false);
    if req.path == "/events" && is_ws {
        serve_ws(stream, req, api).await;
        return;
    }
    if req.method == "POST" && req.path == "/api" {
        let reply = match serde_json::from_slice::<ApiRequest>(&req.body) {
            Ok(r) => api.backend.dispatch(r),
            Err(_) => ApiResponse {
                ok: false,
                data: serde_json::json!({"error": "bad request body"}),
            },
        };
        let value =
            serde_json::to_value(&reply).unwrap_or_else(|_| serde_json::json!({"ok": false}));
        let resp = HttpResponse::json(if reply.ok { 200 } else { 400 }, &value);
        let _ = stream.write_all(&encode_response(&resp)).await;
        return;
    }
    let resp = HttpResponse::json(404, &serde_json::json!({"ok": false}));
    let _ = stream.write_all(&encode_response(&resp)).await;
}

async fn serve_ws(mut stream: TcpStream, req: ParsedRequest, api: &RemoteApi) {
    let Some(key) = req.header("sec-websocket-key").map(str::to_string) else {
        let resp = HttpResponse::json(400, &serde_json::json!({"ok": false}));
        let _ = stream.write_all(&encode_response(&resp)).await;
        return;
    };
    let accept = ws_accept(&key);
    let handshake = format!(
        "HTTP/1.1 101 Switching Protocols\r\nupgrade: websocket\r\nconnection: Upgrade\r\nsec-websocket-accept: {accept}\r\n\r\n"
    );
    if stream.write_all(handshake.as_bytes()).await.is_err() {
        return;
    }
    let (mut rd, mut wr) = tokio::io::split(stream);
    let mut rx = api.backend.subscribe();
    loop {
        tokio::select! {
            ev = rx.recv() => {
                match ev {
                    Ok(ev) => {
                        let line = serde_json::to_string(&ev).unwrap_or_default();
                        if wr.write_all(&ws_frame(0x1, line.as_bytes())).await.is_err() {
                            return;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => return,
                }
            }
            frame = read_ws_frame(&mut rd) => {
                match frame {
                    None => return,
                    Some((0x8, payload)) => {
                        // close: echo it back and hang up
                        let _ = wr.write_all(&ws_frame(0x8, &payload)).await;
                        return;
                    }
                    Some((0x9, payload)) => {
                        // ping → pong
                        if wr.write_all(&ws_frame(0xA, &payload)).await.is_err() {
                            return;
                        }
                    }
                    Some(_) => {}
                }
            }
        }
    }
}

pub fn ws_frame(opcode: u8, payload: &[u8]) -> Vec<u8> {
    let mut f = vec![0x80 | opcode];
    if payload.len() <= 125 {
        f.push(payload.len() as u8);
    } else if payload.len() <= 65535 {
        f.push(126);
        f.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    } else {
        f.push(127);
        f.extend_from_slice(&(payload.len() as u64).to_be_bytes());
    }
    f.extend_from_slice(payload);
    f
}

/// Reads one client frame (masked per RFC 6455) and unmasks it.
async fn read_ws_frame<S>(stream: &mut S) -> Option<(u8, Vec<u8>)>
where
    S: tokio::io::AsyncRead + Unpin,
{
    let mut hdr = [0u8; 2];
    stream.read_exact(&mut hdr).await.ok()?;
    let opcode = hdr[0] & 0x0F;
    let masked = hdr[1] & 0x80 != 0;
    let len = match hdr[1] & 0x7F {
        l if l < 126 => usize::from(l),
        126 => {
            let mut b = [0u8; 2];
            stream.read_exact(&mut b).await.ok()?;
            usize::from(u16::from_be_bytes(b))
        }
        _ => {
            let mut b = [0u8; 8];
            stream.read_exact(&mut b).await.ok()?;
            usize::try_from(u64::from_be_bytes(b)).ok()?
        }
    };
    if len > MAX_WS_FRAME {
        return None;
    }
    let key = if masked {
        let mut k = [0u8; 4];
        stream.read_exact(&mut k).await.ok()?;
        Some(k)
    } else {
        None
    };
    let mut payload = vec![0u8; len];
    stream.read_exact(&mut payload).await.ok()?;
    if let Some(k) = key {
        for (i, b) in payload.iter_mut().enumerate() {
            *b ^= k[i % 4];
        }
    }
    Some((opcode, payload))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::supervisor::InstanceState;
    use serde_json::json;

    #[test]
    fn constant_time_eq_works() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"ab", b"abc"));
    }

    #[test]
    fn auth_gate_rejects_and_locks() {
        // zero lockout: the lock expires immediately
        let g = AuthGate::new("secret".into(), 3, Duration::ZERO);
        assert_eq!(g.check(Some("secret")), Ok(()));
        assert_eq!(g.check(Some("wrong")), Err(AuthError::Unauthorized));
        assert_eq!(g.check(Some("wrong")), Err(AuthError::Unauthorized));
        assert!(matches!(g.check(Some("wrong")), Err(AuthError::Locked(_))));
        // zero lockout expires instantly → correct token accepted again
        assert_eq!(g.check(Some("secret")), Ok(()));

        // real lockout: even the correct token is rejected
        let g2 = AuthGate::new("secret".into(), 3, Duration::from_secs(300));
        for _ in 0..3 {
            let _ = g2.check(Some("wrong"));
        }
        assert!(matches!(
            g2.check(Some("secret")),
            Err(AuthError::Locked(_))
        ));
        assert!(matches!(g2.check(None), Err(AuthError::Locked(_))));
    }

    #[test]
    fn bind_policy_requires_opt_in_for_wan() {
        let loopback: SocketAddr = "127.0.0.1:1".parse().unwrap();
        let lan: SocketAddr = "192.168.1.5:1".parse().unwrap();
        assert!(ensure_bind_allowed(loopback, false).is_ok());
        assert!(ensure_bind_allowed(lan, false).is_err());
        assert!(ensure_bind_allowed(lan, true).is_ok());
    }

    #[test]
    fn parse_request_reads_headers_and_body() {
        let raw = b"POST /api?x=1 HTTP/1.1\r\nauthorization: Bearer tok\r\ncontent-length: 7\r\n\r\n{\"a\":1}";
        let r = parse_request(raw).unwrap();
        assert_eq!(r.method, "POST");
        assert_eq!(r.path, "/api");
        assert_eq!(r.query, "x=1");
        assert_eq!(r.header("authorization"), Some("Bearer tok"));
        assert_eq!(r.query_param("x"), Some("1".to_string()));
        assert_eq!(r.body, b"{\"a\":1}");
    }

    #[test]
    fn incomplete_request_is_none() {
        let raw = b"POST /api HTTP/1.1\r\ncontent-length: 10\r\n\r\nshort";
        assert!(parse_request(raw).is_none());
    }

    #[test]
    fn ws_accept_rfc_vector() {
        // RFC 6455 section 1.3 example
        assert_eq!(
            ws_accept("dGhlIHNhbXBsZSBub25jZQ=="),
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );
    }

    #[test]
    fn ws_frame_header_structure() {
        let f = ws_frame(0x1, b"hi");
        assert_eq!(f[0], 0x81);
        assert_eq!(f[1], 2);
        assert_eq!(&f[2..], b"hi");
        // 126..=65535 payload → 16-bit length
        let big = ws_frame(0x1, &[0u8; 200]);
        assert_eq!(big[1], 126);
        assert_eq!(&big[2..4], &(200u16).to_be_bytes());
    }

    #[test]
    fn base64_known_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    struct EchoBackend {
        tx: broadcast::Sender<Event>,
    }

    impl RemoteBackend for EchoBackend {
        fn dispatch(&self, req: ApiRequest) -> ApiResponse {
            ApiResponse {
                ok: true,
                data: json!({"echo": req.command}),
            }
        }
        fn subscribe(&self) -> tokio::sync::broadcast::Receiver<Event> {
            self.tx.subscribe()
        }
    }

    fn test_cfg(token: &str) -> RemoteApiConfig {
        RemoteApiConfig {
            token: token.into(),
            ..RemoteApiConfig::default()
        }
    }

    async fn spawn_api(token: &str, tx: broadcast::Sender<Event>) -> SocketAddr {
        let api = RemoteApi::new(test_cfg(token), Box::new(EchoBackend { tx }));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(api.serve(listener));
        addr
    }

    async fn roundtrip(stream: &mut TcpStream, request: &[u8]) -> String {
        stream.write_all(request).await.unwrap();
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            match stream.read(&mut chunk).await {
                Ok(0) | Err(_) => break,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
            }
        }
        String::from_utf8_lossy(&buf).into_owned()
    }

    #[tokio::test]
    async fn e2e_http_health_commands_lockout() {
        let (tx, _) = broadcast::channel(16);
        let addr = spawn_api("secret", tx).await;

        // /health — no auth required
        let mut c = TcpStream::connect(addr).await.unwrap();
        let resp = roundtrip(&mut c, b"GET /health HTTP/1.1\r\nhost: x\r\n\r\n").await;
        assert!(resp.contains("200 OK"), "got: {resp}");

        // authorized command → echoes command name
        let body = json!({"command": "list_instances", "args": {}}).to_string();
        let req = format!(
            "POST /api HTTP/1.1\r\nauthorization: Bearer secret\r\ncontent-length: {}\r\n\r\n{body}",
            body.len()
        );
        let mut c = TcpStream::connect(addr).await.unwrap();
        let resp = roundtrip(&mut c, req.as_bytes()).await;
        assert!(
            resp.contains("200") && resp.contains("list_instances"),
            "got: {resp}"
        );

        // unauthorized
        let mut c = TcpStream::connect(addr).await.unwrap();
        let bad = b"GET /api HTTP/1.1\r\nauthorization: Bearer nope\r\n\r\n";
        let resp = roundtrip(&mut c, bad).await;
        assert!(resp.contains("401"), "got: {resp}");

        // burn attempts → locked, even with the right token
        for _ in 0..4 {
            let mut c = TcpStream::connect(addr).await.unwrap();
            let _ = roundtrip(
                &mut c,
                b"GET /api HTTP/1.1\r\nauthorization: Bearer nope\r\n\r\n",
            )
            .await;
        }
        let mut c = TcpStream::connect(addr).await.unwrap();
        let resp = roundtrip(
            &mut c,
            b"GET /api HTTP/1.1\r\nauthorization: Bearer secret\r\n\r\n",
        )
        .await;
        assert!(resp.contains("429"), "got: {resp}");
    }

    #[tokio::test]
    async fn e2e_ws_event_stream() {
        let (tx, _) = broadcast::channel(16);
        let addr = spawn_api("secret", tx.clone()).await;

        let mut c = TcpStream::connect(addr).await.unwrap();
        let handshake = "GET /events?token=secret HTTP/1.1\r\nhost: x\r\nupgrade: websocket\r\nsec-websocket-key: dGhlIHNhbXBsZSBub25jZQ==\r\nsec-websocket-version: 13\r\n\r\n";
        c.write_all(handshake.as_bytes()).await.unwrap();

        let mut buf = [0u8; 512];
        let mut resp_buf = Vec::new();
        while !resp_buf.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = c.read(&mut buf).await.unwrap();
            if n == 0 {
                break;
            }
            resp_buf.extend_from_slice(&buf[..n]);
        }
        let resp = String::from_utf8_lossy(&resp_buf);
        assert!(resp.contains("101"), "got: {resp}");
        assert!(resp.contains("s3pPLMBiTxaQ9kYGzzhZRbK+xOo="));

        tokio::time::sleep(Duration::from_millis(50)).await;
        tx.send(Event::StateChanged {
            instance_id: "i1".into(),
            state: InstanceState::Running,
        })
        .unwrap();

        let mut frame = [0u8; 2];
        c.read_exact(&mut frame).await.unwrap();
        assert_eq!(frame[0] & 0x0F, 0x1);
        let len = usize::from(frame[1] & 0x7F);
        let mut payload = vec![0u8; len];
        c.read_exact(&mut payload).await.unwrap();
        let text = String::from_utf8(payload).unwrap();
        assert!(text.contains("StateChanged"), "got: {text}");

        let mut ping = vec![0x89, 0x81];
        ping.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
        ping.push(b'x');
        c.write_all(&ping).await.unwrap();
        let mut pong_hdr = [0u8; 2];
        c.read_exact(&mut pong_hdr).await.unwrap();
        assert_eq!(pong_hdr[0] & 0x0F, 0xA);
        assert_eq!(pong_hdr[1] & 0x7F, 1);
    }
}
