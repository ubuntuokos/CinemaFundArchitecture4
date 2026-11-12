//! A minimal WebSocket client (RFC 6455) over a plain TCP stream: enough for obs-websocket on
//! the same machine. Text messages only; pings are answered; every read and write has a
//! timeout, and messages are capped in size.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use base64::Engine as _;

/// The longest message accepted from the server.
pub const MAX_MESSAGE: usize = 16 * 1024 * 1024;
const GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

#[derive(Debug)]
pub enum WsError {
    Io(std::io::Error),
    Handshake(String),
    Protocol(String),
    Closed,
}

impl std::fmt::Display for WsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WsError::Io(e) => write!(f, "{e}"),
            WsError::Handshake(m) => write!(f, "WebSocket handshake failed: {m}"),
            WsError::Protocol(m) => write!(f, "WebSocket protocol error: {m}"),
            WsError::Closed => write!(f, "the connection was closed"),
        }
    }
}

impl std::error::Error for WsError {}

impl From<std::io::Error> for WsError {
    fn from(e: std::io::Error) -> Self {
        WsError::Io(e)
    }
}

/// Pseudo-random bytes for the handshake key and frame masks (RFC 6455 asks for unpredictable
/// masks against proxies caching; on a loopback connection this is plenty).
fn random_bytes<const N: usize>() -> [u8; N] {
    use std::hash::{BuildHasher, Hasher};
    let mut out = [0u8; N];
    for chunk in out.chunks_mut(8) {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
        let v = h.finish().to_le_bytes();
        chunk.copy_from_slice(&v[..chunk.len()]);
    }
    out
}

/// The accept value a server must answer `key` with.
pub fn accept_key(key: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(crate::sha1::sha1(format!("{key}{GUID}").as_bytes()))
}

pub struct WsClient {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
}

impl WsClient {
    /// Connect to `ws://host:port/` with `timeout` for connecting and for each read and write.
    pub fn connect(host: &str, port: u16, timeout: Duration) -> Result<WsClient, WsError> {
        let addr = (host, port).to_socket_addrs()?.next().ok_or_else(|| WsError::Handshake(format!("no address for {host}")))?;
        let stream = TcpStream::connect_timeout(&addr, timeout)?;
        stream.set_read_timeout(Some(timeout))?;
        stream.set_write_timeout(Some(timeout))?;
        stream.set_nodelay(true)?;
        let key = base64::engine::general_purpose::STANDARD.encode(random_bytes::<16>());
        let mut writer = stream.try_clone()?;
        write!(
            writer,
            "GET / HTTP/1.1\r\nHost: {host}:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Protocol: obswebsocket.json\r\n\r\n"
        )?;
        let mut reader = BufReader::new(stream);
        let mut status = String::new();
        read_line_capped(&mut reader, &mut status)?;
        if !status.starts_with("HTTP/1.1 101") {
            return Err(WsError::Handshake(status.trim().to_string()));
        }
        let mut accept = None;
        for _ in 0..100 {
            let mut line = String::new();
            read_line_capped(&mut reader, &mut line)?;
            let line = line.trim_end();
            if line.is_empty() {
                break;
            }
            if let Some((k, v)) = line.split_once(':')
                && k.trim().eq_ignore_ascii_case("sec-websocket-accept")
            {
                accept = Some(v.trim().to_string());
            }
        }
        if accept.as_deref() != Some(accept_key(&key).as_str()) {
            return Err(WsError::Handshake("the server's Sec-WebSocket-Accept does not match".into()));
        }
        Ok(WsClient { reader, writer })
    }

    /// Send a text message (masked, as a client must).
    pub fn send_text(&mut self, text: &str) -> Result<(), WsError> {
        self.send_frame(0x1, text.as_bytes())
    }

    fn send_frame(&mut self, opcode: u8, payload: &[u8]) -> Result<(), WsError> {
        let mut frame = Vec::with_capacity(payload.len() + 14);
        frame.push(0x80 | opcode);
        let len = payload.len();
        if len < 126 {
            frame.push(0x80 | len as u8);
        } else if len <= 0xFFFF {
            frame.push(0x80 | 126);
            frame.extend_from_slice(&(len as u16).to_be_bytes());
        } else {
            frame.push(0x80 | 127);
            frame.extend_from_slice(&(len as u64).to_be_bytes());
        }
        let mask = random_bytes::<4>();
        frame.extend_from_slice(&mask);
        frame.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
        self.writer.write_all(&frame)?;
        Ok(())
    }

    /// The next text message; pings are answered on the way, a close ends with [`WsError::Closed`].
    pub fn read_text(&mut self) -> Result<String, WsError> {
        let mut message: Vec<u8> = Vec::new();
        loop {
            let mut head = [0u8; 2];
            self.reader.read_exact(&mut head)?;
            let fin = head[0] & 0x80 != 0;
            let opcode = head[0] & 0x0F;
            let masked = head[1] & 0x80 != 0;
            let mut len = u64::from(head[1] & 0x7F);
            if len == 126 {
                let mut b = [0u8; 2];
                self.reader.read_exact(&mut b)?;
                len = u64::from(u16::from_be_bytes(b));
            } else if len == 127 {
                let mut b = [0u8; 8];
                self.reader.read_exact(&mut b)?;
                len = u64::from_be_bytes(b);
            }
            if len as usize > MAX_MESSAGE || message.len() + len as usize > MAX_MESSAGE {
                return Err(WsError::Protocol(format!("message larger than {MAX_MESSAGE} bytes")));
            }
            let mask = if masked {
                let mut m = [0u8; 4];
                self.reader.read_exact(&mut m)?;
                Some(m)
            } else {
                None
            };
            let mut payload = vec![0u8; len as usize];
            self.reader.read_exact(&mut payload)?;
            if let Some(m) = mask {
                for (i, b) in payload.iter_mut().enumerate() {
                    *b ^= m[i % 4];
                }
            }
            match opcode {
                0x0 | 0x1 => {
                    message.extend_from_slice(&payload);
                    if fin {
                        return String::from_utf8(message).map_err(|_| WsError::Protocol("a text message is not UTF-8".into()));
                    }
                }
                0x2 => return Err(WsError::Protocol("binary messages are not expected".into())),
                0x8 => {
                    let _ = self.send_frame(0x8, &[]);
                    return Err(WsError::Closed);
                }
                0x9 => self.send_frame(0xA, &payload)?,
                0xA => {}
                other => return Err(WsError::Protocol(format!("unknown opcode {other}"))),
            }
        }
    }

    pub fn close(&mut self) {
        let _ = self.send_frame(0x8, &[]);
        let _ = self.writer.shutdown(std::net::Shutdown::Both);
    }
}

fn read_line_capped(r: &mut BufReader<TcpStream>, out: &mut String) -> Result<(), WsError> {
    let n = r.by_ref().take(8 * 1024).read_line(out)?;
    if n == 0 {
        return Err(WsError::Closed);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::accept_key;

    #[test]
    fn accept_key_of_the_rfc_example() {
        // RFC 6455 §1.3
        assert_eq!(accept_key("dGhlIHNhbXBsZSBub25jZQ=="), "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
    }
}
