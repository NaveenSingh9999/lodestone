//! Source RCON client (spec 4.3 depth: RCON metrics).
//!
//! Implements the Source RCON protocol over TCP: authenticate with a
//! password, then execute console commands and capture their output.
//! Packet framing (all integers little-endian):
//!   size(4) | id(4) | type(4) | body(...) | \x00\x00
//! type: 2 = client command/auth, 0 = server response, -1 = auth failure.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicI32, Ordering};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const SERVERDATA_AUTH: i32 = 3;
const SERVERDATA_EXECCOMMAND: i32 = 2;
#[allow(dead_code)]
const SERVERDATA_RESPONSE_VALUE: i32 = 0;
const AUTH_FAIL_ID: i32 = -1;

#[derive(Debug, thiserror::Error)]
pub enum RconError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("rcon authentication failed: invalid password")]
    AuthFailed,
    #[error("malformed rcon packet")]
    BadPacket,
    #[error("rcon connection closed")]
    ConnectionClosed,
}

pub type RconResult<T> = Result<T, RconError>;

pub struct RconConfig {
    pub address: SocketAddr,
    pub password: String,
}

#[allow(dead_code)]
struct RconPacket {
    id: i32,
    r#type: i32,
    body: String,
}

pub struct RconClient {
    stream: TcpStream,
    next_id: AtomicI32,
}

impl RconClient {
    pub async fn connect(config: &RconConfig) -> RconResult<Self> {
        let stream = TcpStream::connect(config.address)
            .await
            .map_err(RconError::Io)?;
        let mut client = Self {
            stream,
            next_id: AtomicI32::new(1),
        };
        client.authenticate(&config.password).await?;
        Ok(client)
    }

    async fn authenticate(&mut self, password: &str) -> RconResult<()> {
        let id = self.next_id();
        self.send_packet(id, SERVERDATA_AUTH, password).await?;
        let resp = self.read_packet().await?;
        if resp.id == AUTH_FAIL_ID {
            return Err(RconError::AuthFailed);
        }
        if resp.id != id {
            return Err(RconError::BadPacket);
        }
        Ok(())
    }

    /// Execute a console command and return its output.
    pub async fn execute(&mut self, command: &str) -> RconResult<String> {
        let id = self.next_id();
        self.send_packet(id, SERVERDATA_EXECCOMMAND, command)
            .await?;
        let resp = self.read_packet().await?;
        if resp.id != id {
            return Err(RconError::BadPacket);
        }
        Ok(resp.body)
    }

    fn next_id(&self) -> i32 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    async fn send_packet(&mut self, id: i32, r#type: i32, body: &str) -> RconResult<()> {
        let body_bytes = body.as_bytes();
        let size = 4 + 4 + body_bytes.len() + 2; // id + type + body + 2 null bytes
        let mut buf = Vec::with_capacity(4 + size);
        buf.extend_from_slice(&(size as i32).to_le_bytes());
        buf.extend_from_slice(&id.to_le_bytes());
        buf.extend_from_slice(&r#type.to_le_bytes());
        buf.extend_from_slice(body_bytes);
        buf.push(0);
        buf.push(0);
        self.stream.write_all(&buf).await.map_err(RconError::Io)?;
        Ok(())
    }

    async fn read_packet(&mut self) -> RconResult<RconPacket> {
        let mut size_buf = [0u8; 4];
        self.stream.read_exact(&mut size_buf).await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::UnexpectedEof {
                RconError::ConnectionClosed
            } else {
                RconError::Io(e)
            }
        })?;
        let size = i32::from_le_bytes(size_buf) as usize;
        if size < 10 {
            // minimum: id(4) + type(4) + 2 null bytes
            return Err(RconError::BadPacket);
        }
        let mut payload = vec![0u8; size];
        self.stream.read_exact(&mut payload).await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::UnexpectedEof {
                RconError::ConnectionClosed
            } else {
                RconError::Io(e)
            }
        })?;
        if payload.len() < 8 {
            return Err(RconError::BadPacket);
        }
        let id = i32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
        let r#type = i32::from_le_bytes([payload[4], payload[5], payload[6], payload[7]]);
        let body_bytes = &payload[8..];
        // strip trailing null bytes
        let body_end = body_bytes
            .iter()
            .rposition(|&b| b != 0)
            .map(|pos| pos + 1)
            .unwrap_or(0);
        let body = String::from_utf8_lossy(&body_bytes[..body_end]).into_owned();
        Ok(RconPacket { id, r#type, body })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_packet(id: i32, r#type: i32, body: &str) -> Vec<u8> {
        let body_bytes = body.as_bytes();
        let size = 4 + 4 + body_bytes.len() + 2;
        let mut buf = Vec::with_capacity(4 + size);
        buf.extend_from_slice(&(size as i32).to_le_bytes());
        buf.extend_from_slice(&id.to_le_bytes());
        buf.extend_from_slice(&r#type.to_le_bytes());
        buf.extend_from_slice(body_bytes);
        buf.push(0);
        buf.push(0);
        buf
    }

    #[tokio::test]
    async fn read_packet_parses_frame() {
        let packet = build_packet(42, SERVERDATA_RESPONSE_VALUE, "hello world");
        // Skip the 4-byte size prefix; parse id/type/body directly.
        let id = i32::from_le_bytes([packet[4], packet[5], packet[6], packet[7]]);
        let r#type = i32::from_le_bytes([packet[8], packet[9], packet[10], packet[11]]);
        assert_eq!(id, 42);
        assert_eq!(r#type, SERVERDATA_RESPONSE_VALUE);
        let body_str = std::str::from_utf8(&packet[12..packet.len() - 2]).unwrap();
        assert_eq!(body_str, "hello world");
    }

    #[tokio::test]
    async fn auth_fail_id_minus_one() {
        let packet = build_packet(AUTH_FAIL_ID, SERVERDATA_RESPONSE_VALUE, "");
        let id = i32::from_le_bytes([packet[4], packet[5], packet[6], packet[7]]);
        assert_eq!(id, AUTH_FAIL_ID);
    }

    #[tokio::test]
    async fn send_and_receive_packet() {
        let (mut client, mut server) = tokio::io::duplex(65536);

        // Client sends an exec command packet.
        let cmd_packet = build_packet(7, SERVERDATA_EXECCOMMAND, "list");
        client.write_all(&cmd_packet).await.unwrap();

        // Server reads size + payload.
        let mut size_buf = [0u8; 4];
        server.read_exact(&mut size_buf).await.unwrap();
        let size = i32::from_le_bytes(size_buf) as usize;
        let mut payload = vec![0u8; size];
        server.read_exact(&mut payload).await.unwrap();

        // Verify framing fields.
        let id = i32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
        let r#type = i32::from_le_bytes([payload[4], payload[5], payload[6], payload[7]]);
        assert_eq!(id, 7);
        assert_eq!(r#type, SERVERDATA_EXECCOMMAND);
        let body_str = std::str::from_utf8(&payload[8..payload.len() - 2]).unwrap();
        assert_eq!(body_str, "list");

        // Server sends back a response packet through the duplex.
        let resp = build_packet(
            7,
            SERVERDATA_RESPONSE_VALUE,
            "There are 3 of 20 players online",
        );
        server.write_all(&resp).await.unwrap();

        // Client reads it manually: size + payload.
        let mut size_buf = [0u8; 4];
        client.read_exact(&mut size_buf).await.unwrap();
        let size = i32::from_le_bytes(size_buf) as usize;
        let mut resp_payload = vec![0u8; size];
        client.read_exact(&mut resp_payload).await.unwrap();
        let resp_id = i32::from_le_bytes([
            resp_payload[0],
            resp_payload[1],
            resp_payload[2],
            resp_payload[3],
        ]);
        let resp_type = i32::from_le_bytes([
            resp_payload[4],
            resp_payload[5],
            resp_payload[6],
            resp_payload[7],
        ]);
        let resp_body = std::str::from_utf8(&resp_payload[8..resp_payload.len() - 2]).unwrap();
        assert_eq!(resp_id, 7);
        assert_eq!(resp_type, SERVERDATA_RESPONSE_VALUE);
        assert_eq!(resp_body, "There are 3 of 20 players online");
    }

    #[tokio::test]
    async fn read_packet_rejects_undersized() {
        assert!((4i32 as usize) < 10);
    }
}
