use super::obfs::random;
use std::io;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const OP_CONT: u8 = 0x0;
pub const OP_TEXT: u8 = 0x1;
pub const OP_BINARY: u8 = 0x2;
pub const OP_CLOSE: u8 = 0x8;
pub const OP_PING: u8 = 0x9;
pub const OP_PONG: u8 = 0xa;
const MAX_MESSAGE: usize = 16 * 1024 * 1024;
const MAX_HEADER: usize = 16 * 1024;

#[derive(Debug)]
pub enum HandshakeError {
    Io(io::Error),
    Status(u16, String),
}

impl std::fmt::Display for HandshakeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HandshakeError::Io(e) => write!(f, "{e}"),
            HandshakeError::Status(code, line) => write!(f, "HTTP {code}: {line}"),
        }
    }
}

impl From<io::Error> for HandshakeError {
    fn from(e: io::Error) -> Self {
        HandshakeError::Io(e)
    }
}

fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in data.chunks(3) {
        let b = [c[0], *c.get(1).unwrap_or(&0), *c.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for i in 0..4 {
            if i <= c.len() {
                out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

pub fn request(host: &str, path: &str) -> String {
    format!(
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {}\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Protocol: binary\r\n\r\n",
        base64(&random::<16>())
    )
}

pub async fn handshake<S: AsyncRead + AsyncWrite + Unpin>(s: &mut S, host: &str, path: &str) -> Result<(), HandshakeError> {
    s.write_all(request(host, path).as_bytes()).await?;
    s.flush().await?;
    let mut head = Vec::new();
    let mut b = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() > MAX_HEADER {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "слишком длинный ответ").into());
        }
        if s.read(&mut b).await? == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "соединение закрыто при рукопожатии").into());
        }
        head.push(b[0]);
    }
    let text = String::from_utf8_lossy(&head);
    let line = text.lines().next().unwrap_or("").to_string();
    let code = line.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    if code == 101 { Ok(()) } else { Err(HandshakeError::Status(code, line)) }
}

pub fn frame(opcode: u8, data: &[u8]) -> Vec<u8> {
    let len = data.len();
    let mut out = Vec::with_capacity(len + 14);
    out.push(0x80 | opcode);
    if len < 126 {
        out.push(0x80 | len as u8);
    } else if len < 65536 {
        out.push(0x80 | 126);
        out.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        out.push(0x80 | 127);
        out.extend_from_slice(&(len as u64).to_be_bytes());
    }
    let mask = random::<4>();
    out.extend_from_slice(&mask);
    out.extend(data.iter().enumerate().map(|(i, b)| b ^ mask[i & 3]));
    out
}

pub enum Event {
    Data(Vec<u8>),
    Ping(Vec<u8>),
    Close(Vec<u8>),
}

pub struct Reader<R> {
    inner: R,
    frag: Vec<u8>,
}

impl<R: AsyncRead + Unpin> Reader<R> {
    pub fn new(inner: R) -> Self {
        Reader { inner, frag: Vec::new() }
    }

    async fn read_frame(&mut self) -> io::Result<(u8, Vec<u8>, bool)> {
        let mut hdr = [0u8; 2];
        self.inner.read_exact(&mut hdr).await?;
        let fin = hdr[0] & 0x80 != 0;
        let opcode = hdr[0] & 0x0f;
        let mut len = (hdr[1] & 0x7f) as u64;
        if len == 126 {
            let mut b = [0u8; 2];
            self.inner.read_exact(&mut b).await?;
            len = u16::from_be_bytes(b) as u64;
        } else if len == 127 {
            let mut b = [0u8; 8];
            self.inner.read_exact(&mut b).await?;
            len = u64::from_be_bytes(b);
        }
        if len > MAX_MESSAGE as u64 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "слишком большой кадр WebSocket"));
        }
        let mask = if hdr[1] & 0x80 != 0 {
            let mut m = [0u8; 4];
            self.inner.read_exact(&mut m).await?;
            Some(m)
        } else {
            None
        };
        let mut payload = vec![0u8; len as usize];
        self.inner.read_exact(&mut payload).await?;
        if let Some(m) = mask {
            for (i, b) in payload.iter_mut().enumerate() {
                *b ^= m[i & 3];
            }
        }
        Ok((opcode, payload, fin))
    }

    pub async fn next(&mut self) -> io::Result<Event> {
        loop {
            let (op, payload, fin) = self.read_frame().await?;
            match op {
                OP_CLOSE => return Ok(Event::Close(payload.into_iter().take(2).collect())),
                OP_PING => return Ok(Event::Ping(payload)),
                OP_CONT | OP_TEXT | OP_BINARY => {
                    if fin && self.frag.is_empty() {
                        return Ok(Event::Data(payload));
                    }
                    self.frag.extend_from_slice(&payload);
                    if self.frag.len() > MAX_MESSAGE {
                        return Err(io::Error::new(io::ErrorKind::InvalidData, "слишком большое сообщение WebSocket"));
                    }
                    if fin {
                        return Ok(Event::Data(std::mem::take(&mut self.frag)));
                    }
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_reference() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(&[0xff; 16]), "/////////////////////w==");
    }

    #[test]
    fn request_has_exact_headers() {
        let r = request("kws2.web.telegram.org", "/apiws");
        assert!(r.starts_with("GET /apiws HTTP/1.1\r\nHost: kws2.web.telegram.org\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: "));
        assert!(r.ends_with("==\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Protocol: binary\r\n\r\n"));
    }

    fn unmask(f: &[u8]) -> (u8, Vec<u8>) {
        let (len, at) = match f[1] & 0x7f {
            126 => (u16::from_be_bytes([f[2], f[3]]) as usize, 4),
            127 => (u64::from_be_bytes(f[2..10].try_into().unwrap()) as usize, 10),
            n => (n as usize, 2),
        };
        let m = &f[at..at + 4];
        (f[0], f[at + 4..at + 4 + len].iter().enumerate().map(|(i, b)| b ^ m[i & 3]).collect())
    }

    #[test]
    fn frames_use_all_length_forms_and_mask() {
        for n in [0usize, 125, 126, 65535, 65536] {
            let data: Vec<u8> = (0..n).map(|i| i as u8).collect();
            let f = frame(OP_BINARY, &data);
            assert_eq!(f[1] & 0x80, 0x80);
            assert_eq!(unmask(&f), (0x82, data));
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn reader_reassembles_and_reports_control() {
        let mut wire = Vec::new();
        wire.extend([0x02, 3, 1, 2, 3]);
        wire.extend([0x89, 1, 7]);
        wire.extend([0x80, 2, 4, 5]);
        wire.extend([0x82, 0x81, 1, 0, 0, 0, 9 ^ 1]);
        wire.extend([0x88, 4, 3, 232, b'o', b'k']);
        let mut r = Reader::new(&wire[..]);
        assert!(matches!(r.next().await.unwrap(), Event::Ping(p) if p == vec![7]));
        assert!(matches!(r.next().await.unwrap(), Event::Data(d) if d == vec![1, 2, 3, 4, 5]));
        assert!(matches!(r.next().await.unwrap(), Event::Data(d) if d == vec![9]));
        assert!(matches!(r.next().await.unwrap(), Event::Close(c) if c == vec![3, 232]));
    }
}
