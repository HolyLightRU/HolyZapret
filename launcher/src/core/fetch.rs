use std::io::Read;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{RecvTimeoutError, channel};
use std::time::Duration;

const STALL: Duration = Duration::from_secs(30);
const ATTEMPTS: usize = 6;
const CHUNK: usize = 64 * 1024;

#[derive(Default)]
pub struct Progress {
    pub done: AtomicU64,
    pub total: AtomicU64,
}

impl Progress {
    pub fn reset(&self) {
        self.done.store(0, Ordering::Relaxed);
        self.total.store(0, Ordering::Relaxed);
    }

    pub fn get(&self) -> (u64, u64) {
        (self.done.load(Ordering::Relaxed), self.total.load(Ordering::Relaxed))
    }
}

enum Piece {
    Start { restart: bool, total: Option<u64> },
    Data(Vec<u8>),
    End,
    Fail(String),
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(20)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .user_agent("HolyZapret-Launcher")
        .build()
        .into()
}

fn header_u64(resp: &ureq::http::Response<ureq::Body>, name: &str) -> Option<u64> {
    resp.headers().get(name)?.to_str().ok()?.trim().parse().ok()
}

fn content_range_total(resp: &ureq::http::Response<ureq::Body>) -> Option<u64> {
    resp.headers().get("content-range")?.to_str().ok()?.rsplit('/').next()?.trim().parse().ok()
}

fn attempt(url: String, offset: u64, limit: u64, tx: std::sync::mpsc::Sender<Piece>) {
    let mut req = agent().get(&url);
    if offset > 0 {
        req = req.header("Range", format!("bytes={offset}-"));
    }
    let mut resp = match req.call() {
        Ok(r) => r,
        Err(e) => {
            let _ = tx.send(Piece::Fail(e.to_string()));
            return;
        }
    };
    let partial = resp.status().as_u16() == 206;
    let total = if partial { content_range_total(&resp) } else { header_u64(&resp, "content-length") };
    if tx.send(Piece::Start { restart: offset > 0 && !partial, total }).is_err() {
        return;
    }
    let mut reader = resp.body_mut().with_config().limit(limit.saturating_add(1)).reader();
    let mut buf = vec![0u8; CHUNK];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => {
                let _ = tx.send(Piece::End);
                return;
            }
            Ok(n) => {
                if tx.send(Piece::Data(buf[..n].to_vec())).is_err() {
                    return;
                }
            }
            Err(e) => {
                let _ = tx.send(Piece::Fail(e.to_string()));
                return;
            }
        }
    }
}

pub fn download(url: &str, limit: u64, progress: Option<&Progress>) -> anyhow::Result<Vec<u8>> {
    let mut data: Vec<u8> = Vec::new();
    let mut total: Option<u64> = None;
    let mut last = String::new();
    for n in 0..ATTEMPTS {
        if total.is_some_and(|t| data.len() as u64 >= t) {
            return Ok(data);
        }
        if n > 0 {
            std::thread::sleep(Duration::from_secs(2));
        }
        let (tx, rx) = channel();
        let (u, offset) = (url.to_string(), data.len() as u64);
        std::thread::spawn(move || attempt(u, offset, limit, tx));
        loop {
            match rx.recv_timeout(STALL) {
                Ok(Piece::Start { restart, total: t }) => {
                    if restart {
                        data.clear();
                    }
                    if t.is_some() {
                        total = t;
                    }
                    if let (Some(p), Some(t)) = (progress, total) {
                        p.total.store(t, Ordering::Relaxed);
                    }
                }
                Ok(Piece::Data(chunk)) => {
                    data.extend_from_slice(&chunk);
                    if data.len() as u64 > limit {
                        anyhow::bail!("файл больше {} МБ, загрузка остановлена", limit / 1024 / 1024);
                    }
                    if let Some(p) = progress {
                        p.done.store(data.len() as u64, Ordering::Relaxed);
                    }
                }
                Ok(Piece::End) => {
                    if total.is_none_or(|t| data.len() as u64 >= t) {
                        return Ok(data);
                    }
                    last = "соединение оборвалось".into();
                    break;
                }
                Ok(Piece::Fail(e)) => {
                    last = e;
                    break;
                }
                Err(RecvTimeoutError::Timeout) => {
                    last = "загрузка зависла".into();
                    break;
                }
                Err(RecvTimeoutError::Disconnected) => {
                    last = "загрузка прервалась".into();
                    break;
                }
            }
        }
    }
    anyhow::bail!("не удалось скачать с GitHub ({last}). Включите защиту HolyZapret или WARP и повторите")
}

pub fn with_retry<T>(tries: usize, f: impl Fn() -> anyhow::Result<T>) -> anyhow::Result<T> {
    let mut last = None;
    for n in 0..tries.max(1) {
        if n > 0 {
            std::thread::sleep(Duration::from_secs(2));
        }
        match f() {
            Ok(v) => return Ok(v),
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| anyhow::anyhow!("не удалось")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn server(data: Vec<u8>, stall_after: Option<usize>) -> u16 {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let mut first = true;
            for s in l.incoming().flatten() {
                let mut s = s;
                let mut req = [0u8; 2048];
                let n = s.read(&mut req).unwrap_or(0);
                let text = String::from_utf8_lossy(&req[..n]).to_lowercase();
                let from: usize = text.split("range: bytes=").nth(1).and_then(|r| r.split('-').next()).and_then(|v| v.trim().parse().ok()).unwrap_or(0);
                let body = &data[from..];
                let head = if from > 0 {
                    format!("HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\n\r\n", body.len(), from, data.len() - 1, data.len())
                } else {
                    format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len())
                };
                let _ = s.write_all(head.as_bytes());
                match stall_after.filter(|_| first) {
                    Some(cut) => {
                        let _ = s.write_all(&body[..cut.min(body.len())]);
                        let _ = s.flush();
                        drop(s);
                    }
                    None => {
                        let _ = s.write_all(body);
                    }
                }
                first = false;
            }
        });
        port
    }

    #[test]
    fn resumes_after_broken_connection() {
        let data: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        let port = server(data.clone(), Some(100_000));
        let p = Progress::default();
        let got = download(&format!("http://127.0.0.1:{port}/f.zip"), 10 * 1024 * 1024, Some(&p)).unwrap();
        assert_eq!(got, data);
        assert_eq!(p.get(), (300_000, 300_000));
    }

    #[test]
    fn plain_download_and_limit() {
        let data = vec![7u8; 5000];
        let port = server(data.clone(), None);
        assert_eq!(download(&format!("http://127.0.0.1:{port}/a"), 1_000_000, None).unwrap(), data);
        let port = server(vec![1u8; 5000], None);
        assert!(download(&format!("http://127.0.0.1:{port}/a"), 1000, None).is_err());
    }
}
