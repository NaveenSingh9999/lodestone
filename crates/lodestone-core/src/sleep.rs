//! Sleep-when-empty, wake-on-join proxy (spec 4.12).
//!
//! Reimplemented from the Minecraft Java server-list-ping and login
//! protocols. While the managed server is asleep this proxy owns the
//! public port: status pings get a configurable "sleeping" MOTD, and a
//! join attempt wakes the server and kicks the client with a retry
//! message. While the server runs the proxy relays bytes, optionally
//! prepending a PROXY protocol v1 header so the server sees real
//! client IPs. An idle watcher polls the local server's player count
//! and sleeps it after N minutes with no players.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// Maximum buffered bytes for a sleeping-client session before it is dropped.
const MAX_SESSION_BUFFER: usize = 65536;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[error("bad minecraft protocol data: {0}")]
pub struct ProtoError(pub &'static str);

pub type ProtoResult<T> = Result<T, ProtoError>;

pub fn write_varint(buf: &mut Vec<u8>, mut value: u32) {
    loop {
        if value & !0x7F == 0 {
            buf.push(value as u8);
            return;
        }
        buf.push(((value & 0x7F) | 0x80) as u8);
        value >>= 7;
    }
}

/// Returns the decoded value and the number of bytes consumed.
pub fn read_varint(buf: &[u8]) -> ProtoResult<(i32, usize)> {
    let mut value: u32 = 0;
    for (i, &b) in buf.iter().enumerate() {
        if i == 5 {
            return Err(ProtoError("varint too long"));
        }
        value |= u32::from(b & 0x7F) << (7 * i);
        if b & 0x80 == 0 {
            return Ok((value as i32, i + 1));
        }
    }
    Err(ProtoError("varint truncated"))
}

/// Encodes one framed packet: varint length, varint id, payload.
pub fn encode_packet(id: i32, payload: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(5 + payload.len());
    write_varint(&mut body, id as u32);
    body.extend_from_slice(payload);
    let mut out = Vec::with_capacity(5 + body.len());
    write_varint(&mut out, body.len() as u32);
    out.extend_from_slice(&body);
    out
}

/// Drains the next complete frame from `buf`, returning its id and payload.
fn next_frame(buf: &mut Vec<u8>) -> Option<(i32, Vec<u8>)> {
    let (len, hdr) = read_varint(buf).ok()?;
    if len < 0 {
        return None;
    }
    let end = hdr + len as usize;
    if buf.len() < end {
        return None;
    }
    let (id, id_len) = read_varint(&buf[hdr..end]).ok()?;
    let payload = buf[hdr + id_len..end].to_vec();
    buf.drain(..end);
    Some((id, payload))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handshake {
    pub protocol_version: i32,
    pub server_address: String,
    pub server_port: u16,
    pub next_state: i32,
}

pub fn parse_handshake(payload: &[u8]) -> ProtoResult<Handshake> {
    let (protocol_version, mut pos) =
        read_varint(payload).map_err(|_| ProtoError("protocol varint"))?;
    let (addr_len, used) = read_varint(&payload[pos..]).map_err(|_| ProtoError("addr length"))?;
    pos += used;
    let addr_end = pos + addr_len as usize;
    if addr_len < 0 || payload.len() < addr_end {
        return Err(ProtoError("addr out of bounds"));
    }
    let server_address = String::from_utf8_lossy(&payload[pos..addr_end]).into_owned();
    pos = addr_end;
    if pos + 3 > payload.len() {
        return Err(ProtoError("port or next state truncated"));
    }
    let server_port = u16::from_be_bytes([payload[pos], payload[pos + 1]]);
    let (next_state, _) = read_varint(&payload[pos + 2..]).map_err(|_| ProtoError("next state"))?;
    if next_state != 1 && next_state != 2 {
        return Err(ProtoError("next state not status or login"));
    }
    Ok(Handshake {
        protocol_version,
        server_address,
        server_port,
        next_state,
    })
}

pub fn encode_handshake(
    protocol_version: i32,
    server_address: &str,
    server_port: u16,
    next_state: i32,
) -> Vec<u8> {
    let mut payload = Vec::new();
    write_varint(&mut payload, protocol_version as u32);
    write_varint(&mut payload, server_address.len() as u32);
    payload.extend_from_slice(server_address.as_bytes());
    payload.extend_from_slice(&server_port.to_be_bytes());
    write_varint(&mut payload, next_state as u32);
    encode_packet(0, &payload)
}

#[derive(Debug, Clone)]
pub struct SleepConfig {
    pub motd: String,
    pub wake_message: String,
    pub idle_minutes: u64,
    pub proxy_protocol: bool,
    pub protocol_version: i32,
}

impl Default for SleepConfig {
    fn default() -> Self {
        Self {
            motd: "Server is sleeping — join to wake it".to_string(),
            wake_message: "Starting the server, retry shortly".to_string(),
            idle_minutes: 10,
            proxy_protocol: false,
            protocol_version: 767,
        }
    }
}

fn json_string(s: &str) -> String {
    serde_json::Value::String(s.to_string()).to_string()
}

pub fn sleeping_status_json(cfg: &SleepConfig) -> String {
    format!(
        "{{\"version\":{{\"name\":\"Lodestone\",\"protocol\":{}}},\"players\":{{\"max\":0,\"online\":0}},\"description\":{{\"text\":{}}}}}",
        cfg.protocol_version,
        json_string(&cfg.motd)
    )
}

/// PROXY protocol v1 header line (spec 4.12: forward real client IPs).
pub fn proxy_v1_header(src: SocketAddr, dst: SocketAddr) -> String {
    let fam = if src.is_ipv4() { "TCP4" } else { "TCP6" };
    format!(
        "PROXY {fam} {} {} {} {}\r\n",
        src.ip(),
        dst.ip(),
        src.port(),
        dst.port()
    )
}

pub trait WakeHook: Send + Sync {
    fn wake(&self);
}

pub trait SleepHook: Send + Sync {
    fn sleep(&self);
}

/// Per-connection state machine for a client talking to an asleep server.
/// Synchronous and buffered: the async layer just pumps bytes in and out.
pub struct SleepingSession {
    cfg: SleepConfig,
    wake: Arc<dyn WakeHook>,
    buf: Vec<u8>,
    handshake_seen: bool,
    next_state: i32,
    pub woke: bool,
    pub alive: bool,
}

impl SleepingSession {
    pub fn new(cfg: SleepConfig, wake: Arc<dyn WakeHook>) -> Self {
        Self {
            cfg,
            wake,
            buf: Vec::new(),
            handshake_seen: false,
            next_state: 0,
            woke: false,
            alive: true,
        }
    }

    /// Feeds raw bytes from the client; returns bytes to send back.
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<u8> {
        if !self.alive {
            return Vec::new();
        }
        self.buf.extend_from_slice(chunk);
        if self.buf.len() > MAX_SESSION_BUFFER {
            self.alive = false;
            return Vec::new();
        }
        let mut out = Vec::new();
        while let Some((id, payload)) = next_frame(&mut self.buf) {
            if self.handshake_seen && self.next_state == 1 && id == 1 {
                // status ping → echo the pong payload back
                out.extend_from_slice(&encode_packet(1, &payload));
                continue;
            }
            if id != 0 {
                continue;
            }
            if !self.handshake_seen {
                if let Ok(h) = parse_handshake(&payload) {
                    self.handshake_seen = true;
                    self.next_state = h.next_state;
                    if h.next_state == 2 {
                        // join attempt → wake and kick with a retry message
                        self.wake.wake();
                        self.woke = true;
                        let json = format!("{{\"text\":{}}}", json_string(&self.cfg.wake_message));
                        out.extend_from_slice(&encode_packet(0, json.as_bytes()));
                    }
                }
            } else if self.next_state == 1 && payload.is_empty() {
                // status request → sleeping MOTD
                let json = sleeping_status_json(&self.cfg);
                out.extend_from_slice(&encode_packet(0, json.as_bytes()));
            }
        }
        out
    }
}

/// Queries the local server's status ping; returns (online, max).
pub async fn query_status(server: SocketAddr, timeout: Duration) -> ProtoResult<(u32, u32)> {
    let mut stream = match tokio::time::timeout(timeout, TcpStream::connect(server)).await {
        Ok(Ok(s)) => s,
        _ => return Err(ProtoError("connect failed")),
    };
    let hs = encode_handshake(767, &server.ip().to_string(), server.port(), 1);
    let req = encode_packet(0, &[]);
    if tokio::time::timeout(timeout, stream.write_all(&hs))
        .await
        .is_err()
    {
        return Err(ProtoError("handshake write failed"));
    }
    if tokio::time::timeout(timeout, stream.write_all(&req))
        .await
        .is_err()
    {
        return Err(ProtoError("request write failed"));
    }
    let (_, payload) = tokio::time::timeout(timeout, read_frame(&mut stream))
        .await
        .map_err(|_| ProtoError("timeout"))?
        .map_err(|_| ProtoError("read failed"))?;
    let json: serde_json::Value =
        serde_json::from_slice(&payload).map_err(|_| ProtoError("bad status json"))?;
    let online = json["players"]["online"].as_u64().unwrap_or(0) as u32;
    let max = json["players"]["max"].as_u64().unwrap_or(0) as u32;
    Ok((online, max))
}

/// Reads one framed packet from the wire; returns its id and payload.
async fn read_frame(stream: &mut TcpStream) -> std::io::Result<(i32, Vec<u8>)> {
    let mut lenb = Vec::new();
    let mut one = [0u8; 1];
    for _ in 0..5 {
        stream.read_exact(&mut one).await?;
        lenb.push(one[0]);
        if one[0] & 0x80 == 0 {
            break;
        }
    }
    let (len, _) =
        read_varint(&lenb).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let mut frame = vec![0u8; len as usize];
    stream.read_exact(&mut frame).await?;
    let (id, id_len) =
        read_varint(&frame).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    Ok((id, frame[id_len..].to_vec()))
}

pub struct SleepProxy {
    pub bind: SocketAddr,
    pub server: SocketAddr,
    pub cfg: SleepConfig,
    pub running: Arc<AtomicBool>,
    pub wake: Arc<dyn WakeHook>,
}

impl SleepProxy {
    /// Binds the public port and serves forever (spec 4.12).
    pub async fn run(self) -> std::io::Result<()> {
        let listener = TcpListener::bind(self.bind).await?;
        self.serve(listener).await
    }

    /// Serves on an already-bound listener (tests bind an ephemeral port).
    pub async fn serve(self, listener: TcpListener) -> std::io::Result<()> {
        loop {
            let (client, peer) = listener.accept().await?;
            let running = self.running.clone();
            let cfg = self.cfg.clone();
            let wake = self.wake.clone();
            let server = self.server;
            tokio::spawn(async move {
                if running.load(Ordering::Relaxed) {
                    let header = if cfg.proxy_protocol {
                        Some(proxy_v1_header(peer, server))
                    } else {
                        None
                    };
                    passthrough(client, server, header.as_deref()).await;
                } else {
                    serve_sleeping(client, cfg, wake).await;
                }
            });
        }
    }
}

async fn passthrough(mut client: TcpStream, server: SocketAddr, header: Option<&str>) {
    let mut upstream = match TcpStream::connect(server).await {
        Ok(s) => s,
        Err(_) => return,
    };
    if let Some(h) = header
        && upstream.write_all(h.as_bytes()).await.is_err()
    {
        return;
    }
    let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
}

async fn serve_sleeping(mut client: TcpStream, cfg: SleepConfig, wake: Arc<dyn WakeHook>) {
    let mut session = SleepingSession::new(cfg, wake);
    let mut buf = [0u8; 4096];
    loop {
        let n = match client.read(&mut buf).await {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        let out = session.feed(&buf[..n]);
        if !out.is_empty() && client.write_all(&out).await.is_err() {
            return;
        }
        if session.woke {
            let _ = client.shutdown().await;
            return;
        }
        if !session.alive {
            return;
        }
    }
}

/// Tracks emptiness over polls; fires exactly once when the idle
/// threshold is crossed with zero players.
pub struct IdleTracker {
    idle_after: Option<Duration>,
    empty_since: Option<Instant>,
    slept: bool,
}

impl IdleTracker {
    pub fn new(idle_minutes: u64) -> Self {
        Self::with_idle_after((idle_minutes > 0).then(|| Duration::from_secs(idle_minutes * 60)))
    }

    pub fn with_idle_after(idle_after: Option<Duration>) -> Self {
        Self {
            idle_after,
            empty_since: None,
            slept: false,
        }
    }

    /// Feed the latest player count; returns true when the server should sleep.
    pub fn on_player_count(&mut self, online: u32) -> bool {
        match (self.idle_after, online) {
            (Some(after), 0) => {
                let since = self.empty_since.get_or_insert_with(Instant::now);
                if !self.slept && since.elapsed() >= after {
                    self.slept = true;
                    true
                } else {
                    false
                }
            }
            _ => {
                self.empty_since = None;
                self.slept = false;
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    struct CountingWake(Arc<AtomicUsize>);

    impl WakeHook for CountingWake {
        fn wake(&self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn wake_pair() -> (Arc<CountingWake>, Arc<AtomicUsize>) {
        let n = Arc::new(AtomicUsize::new(0));
        (Arc::new(CountingWake(n.clone())), n)
    }

    #[test]
    fn varint_roundtrip() {
        for v in [0u32, 1, 127, 128, 300, 16383, 16384, 2097152, u32::MAX / 2] {
            let mut buf = Vec::new();
            write_varint(&mut buf, v);
            let (decoded, used) = read_varint(&buf).unwrap();
            assert_eq!(decoded as u32, v);
            assert_eq!(used, buf.len());
        }
    }

    #[test]
    fn packet_framing() {
        let p = encode_packet(0, b"hi");
        // length = 1 (id varint) + 2 (payload)
        assert_eq!(p[0], 3);
        let (id, payload) = next_frame(&mut p.clone()).unwrap();
        assert_eq!(id, 0);
        assert_eq!(payload, b"hi");
        assert!(next_frame(&mut Vec::new()).is_none());
    }

    #[test]
    fn handshake_roundtrip() {
        for next in [1, 2] {
            let bytes = encode_handshake(770, "127.0.0.1", 25565, next);
            let (len, hdr) = read_varint(&bytes).unwrap();
            let (id, id_len) = read_varint(&bytes[hdr..]).unwrap();
            assert_eq!(id, 0);
            let hs = parse_handshake(&bytes[hdr + id_len..hdr + len as usize]).unwrap();
            assert_eq!(hs.protocol_version, 770);
            assert_eq!(hs.server_address, "127.0.0.1");
            assert_eq!(hs.server_port, 25565);
            assert_eq!(hs.next_state, next);
        }
    }

    #[test]
    fn handshake_rejects_bad_next_state() {
        let bytes = encode_handshake(770, "x", 25565, 3);
        let (len, hdr) = read_varint(&bytes).unwrap();
        let (_id, id_len) = read_varint(&bytes[hdr..]).unwrap();
        let payload = &bytes[hdr + id_len..hdr + len as usize];
        assert!(parse_handshake(payload).is_err());
    }

    #[test]
    fn sleeping_session_status_reply() {
        let (wake, _count) = wake_pair();
        let mut session = SleepingSession::new(SleepConfig::default(), wake);
        let mut chunk = encode_handshake(770, "s.example", 25565, 1);
        chunk.extend_from_slice(&encode_packet(0, &[]));
        let out = session.feed(&chunk);
        let (id, payload) = next_frame(&mut out.clone()).unwrap();
        assert_eq!(id, 0);
        let json = String::from_utf8(payload).unwrap();
        assert!(json.contains("sleeping"));
        assert!(json.contains("\"online\":0"));
        assert!(!session.woke);
    }

    #[test]
    fn sleeping_session_wake_on_join() {
        let (wake, count) = wake_pair();
        let mut session = SleepingSession::new(SleepConfig::default(), wake);
        let chunk = encode_handshake(770, "s.example", 25565, 2);
        let out = session.feed(&chunk);
        assert_eq!(count.load(Ordering::SeqCst), 1);
        assert!(session.woke);
        let (id, payload) = next_frame(&mut out.clone()).unwrap();
        assert_eq!(id, 0);
        let json = String::from_utf8(payload).unwrap();
        assert!(json.contains("retry shortly"));
    }

    #[test]
    fn sleeping_session_ping_pong() {
        let (wake, _count) = wake_pair();
        let mut session = SleepingSession::new(SleepConfig::default(), wake);
        let mut chunk = encode_handshake(770, "s", 25565, 1);
        chunk.extend_from_slice(&encode_packet(0, &[]));
        chunk.extend_from_slice(&encode_packet(1, &1234567890i64.to_be_bytes()));
        let out = session.feed(&chunk);
        // first frame: status, second: pong
        let mut rest = out.clone();
        let (id0, _) = next_frame(&mut rest).unwrap();
        assert_eq!(id0, 0);
        let (id1, pong) = next_frame(&mut rest).unwrap();
        assert_eq!(id1, 1);
        assert_eq!(pong, 1234567890i64.to_be_bytes());
    }

    #[test]
    fn oversized_session_dropped() {
        let (wake, _count) = wake_pair();
        let mut session = SleepingSession::new(SleepConfig::default(), wake);
        let junk = vec![0x70u8; MAX_SESSION_BUFFER + 1];
        session.feed(&junk);
        assert!(!session.alive);
    }

    #[test]
    fn proxy_header_format() {
        let src: SocketAddr = "192.0.2.1:40000".parse().unwrap();
        let dst: SocketAddr = "192.0.2.2:25565".parse().unwrap();
        assert_eq!(
            proxy_v1_header(src, dst),
            "PROXY TCP4 192.0.2.1 192.0.2.2 40000 25565\r\n"
        );
    }

    #[test]
    fn idle_tracker_fires_once_and_resets() {
        let mut t = IdleTracker::with_idle_after(Some(Duration::from_millis(0)));
        assert!(t.on_player_count(0));
        assert!(!t.on_player_count(0)); // fires exactly once
        assert!(!t.on_player_count(2)); // players joined → reset
        assert!(t.on_player_count(0)); // fresh empty window → fires again
    }

    #[test]
    fn idle_tracker_disabled_when_zero() {
        let mut t = IdleTracker::new(0);
        assert!(!t.on_player_count(0));
        assert!(!t.on_player_count(0));
    }

    #[tokio::test]
    async fn e2e_sleeping_status_and_wake() {
        let (wake, count) = wake_pair();
        let server: SocketAddr = "127.0.0.1:1".parse().unwrap();
        let proxy = SleepProxy {
            bind: "127.0.0.1:0".parse().unwrap(),
            server,
            cfg: SleepConfig::default(),
            running: Arc::new(AtomicBool::new(false)),
            wake,
        };
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let bind_addr = listener.local_addr().unwrap();
        tokio::spawn(async move { proxy.serve(listener).await });

        // status ping while asleep
        let mut c = TcpStream::connect(bind_addr).await.unwrap();
        c.write_all(&encode_handshake(770, "s", 25565, 1))
            .await
            .unwrap();
        c.write_all(&encode_packet(0, &[])).await.unwrap();
        let mut buf = [0u8; 1024];
        let n = c.read(&mut buf).await.unwrap();
        assert!(n > 0);
        let text = String::from_utf8_lossy(&buf[..n]);
        assert!(text.contains("sleeping"), "got: {text}");

        // join attempt while asleep
        let mut c2 = TcpStream::connect(bind_addr).await.unwrap();
        c2.write_all(&encode_handshake(770, "s", 25565, 2))
            .await
            .unwrap();
        let mut buf2 = [0u8; 256];
        let n2 = c2.read(&mut buf2).await.unwrap();
        assert!(n2 > 0);
        let text2 = String::from_utf8_lossy(&buf2[..n2]);
        assert!(text2.contains("retry shortly"), "got: {text2}");
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn e2e_passthrough_and_query_status() {
        // fake minecraft server: speaks status, echoes anything else
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let server_addr = l.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut s, _)) = l.accept().await else { break };
                tokio::spawn(async move {
                    let mut buf = [0u8; 512];
                    let n = s.read(&mut buf).await.unwrap();
                    let body = String::from_utf8_lossy(&buf[..n]);
                    if body.contains("PROXY") {
                        let _ = s.write_all(b"echo-after-proxy").await;
                    } else {
                        let resp = encode_packet(0, br#"{"players":{"online":3,"max":20}}"#);
                        let _ = s.write_all(&resp).await;
                    }
                });
            }
        });

        let (online, max) = query_status(server_addr, Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!((online, max), (3, 20));

        // proxy running → passthrough carries bytes to the server
        let (wake, _count) = wake_pair();
        let proxy = SleepProxy {
            bind: "127.0.0.1:0".parse().unwrap(),
            server: server_addr,
            cfg: SleepConfig {
                proxy_protocol: true,
                ..SleepConfig::default()
            },
            running: Arc::new(AtomicBool::new(true)),
            wake,
        };
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let bind_addr = listener.local_addr().unwrap();
        tokio::spawn(async move { proxy.serve(listener).await });

        let mut c = TcpStream::connect(bind_addr).await.unwrap();
        c.write_all(b"hello").await.unwrap();
        let mut buf = [0u8; 64];
        let n = tokio::time::timeout(Duration::from_secs(5), c.read(&mut buf))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&buf[..n], b"echo-after-proxy");
    }
}
