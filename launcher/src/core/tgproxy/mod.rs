pub mod control;
pub mod obfs;
pub mod route;
pub mod split;
pub mod ws;

use obfs::{Ciphers, INIT_LEN};
use route::{Kind, Routes, Target, Upstream};
use split::Splitter;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio::time::timeout;

pub const DEFAULT_PORT: u16 = 1443;
const INIT_TIMEOUT: Duration = Duration::from_secs(10);
const DRAIN_LIMIT: usize = 64 * 1024;
const BUF: usize = 64 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub port: u16,
    pub secret: [u8; 16],
    pub cloudflare: bool,
    pub cf_domain: String,
}

impl Config {
    pub fn link(&self) -> String {
        format!("tg://proxy?server=127.0.0.1&port={}&secret=dd{}", self.port, hex(&self.secret))
    }
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn parse_secret(s: &str) -> Option<[u8; 16]> {
    let s = s.trim();
    let s = s.strip_prefix("dd").filter(|r| r.len() == 32).unwrap_or(s);
    if s.len() != 32 {
        return None;
    }
    let mut out = [0u8; 16];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

pub fn new_secret() -> [u8; 16] {
    obfs::random()
}

#[derive(Default)]
pub struct Stats {
    pub active: AtomicUsize,
    pub total: AtomicUsize,
    pub via_ws: AtomicUsize,
    pub via_cf: AtomicUsize,
    pub via_tcp: AtomicUsize,
    pub via_warp: AtomicUsize,
    pub failed: AtomicUsize,
    pub bad_secret: AtomicUsize,
    pub up: AtomicU64,
    pub down: AtomicU64,
    pub last_error: Mutex<String>,
    pub last_cf: Mutex<String>,
}

impl Stats {
    fn error(&self, e: impl Into<String>) {
        *self.last_error.lock().unwrap() = e.into();
    }
}

pub struct Handle {
    stop: Option<oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
    pub stats: Arc<Stats>,
    pub config: Config,
}

impl Handle {
    pub fn stop(&mut self) {
        if let Some(s) = self.stop.take() {
            let _ = s.send(());
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        self.stop();
    }
}

struct Shared {
    secret: [u8; 16],
    routes: Routes,
    stats: Arc<Stats>,
}

pub fn start(config: Config) -> anyhow::Result<Handle> {
    let routes = Routes::new(Some(config.cf_domain.as_str()), config.cloudflare)?;
    start_with(config, routes)
}

fn start_with(config: Config, routes: Routes) -> anyhow::Result<Handle> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", config.port))
        .map_err(|e| anyhow::anyhow!("порт {} занят или недоступен: {e}", config.port))?;
    listener.set_nonblocking(true)?;
    let stats = Arc::new(Stats::default());
    let shared = Arc::new(Shared { secret: config.secret, routes, stats: stats.clone() });
    let (tx, rx) = oneshot::channel();
    let thread = std::thread::Builder::new().name("tg-proxy".into()).spawn(move || {
        let Ok(rt) = tokio::runtime::Builder::new_current_thread().enable_all().build() else { return };
        rt.block_on(async move {
            let Ok(listener) = TcpListener::from_std(listener) else { return };
            serve(listener, shared, rx).await;
        });
    })?;
    Ok(Handle { stop: Some(tx), thread: Some(thread), stats, config })
}

async fn serve(listener: TcpListener, shared: Arc<Shared>, mut stop: oneshot::Receiver<()>) {
    loop {
        tokio::select! {
            _ = &mut stop => break,
            r = listener.accept() => {
                if let Ok((s, _)) = r {
                    let sh = shared.clone();
                    tokio::spawn(async move {
                        sh.stats.active.fetch_add(1, Ordering::Relaxed);
                        sh.stats.total.fetch_add(1, Ordering::Relaxed);
                        client(s, &sh).await;
                        sh.stats.active.fetch_sub(1, Ordering::Relaxed);
                    });
                }
            }
        }
    }
}

async fn drain(mut s: TcpStream) {
    let mut buf = [0u8; 4096];
    let mut seen = 0;
    let _ = timeout(INIT_TIMEOUT, async {
        while seen < DRAIN_LIMIT {
            match s.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => seen += n,
            }
        }
    })
    .await;
}

async fn client(mut s: TcpStream, sh: &Shared) {
    let _ = s.set_nodelay(true);
    let mut init = [0u8; INIT_LEN];
    if !matches!(timeout(INIT_TIMEOUT, s.read_exact(&mut init)).await, Ok(Ok(_))) {
        return;
    }
    let Some(hello) = obfs::parse_hello(&init, &sh.secret) else {
        sh.stats.bad_secret.fetch_add(1, Ordering::Relaxed);
        sh.stats.error("Telegram подключился с другим secret: добавьте прокси заново по ссылке");
        drain(s).await;
        return;
    };
    let relay = obfs::relay_init(hello.proto, hello.dc_idx());
    let ciphers = obfs::ciphers(&hello, &sh.secret, &relay);
    let target = Target { dc: hello.dc, media: hello.media, test: hello.test };
    let Some((kind, up)) = sh.routes.connect(&target).await else {
        sh.stats.failed.fetch_add(1, Ordering::Relaxed);
        sh.stats.error(format!("DC{} недоступен: WebSocket, Cloudflare и прямое подключение не отвечают", hello.dc));
        return;
    };
    match &kind {
        Kind::Ws => sh.stats.via_ws.fetch_add(1, Ordering::Relaxed),
        Kind::Cf(d) => {
            *sh.stats.last_cf.lock().unwrap() = d.clone();
            sh.stats.via_cf.fetch_add(1, Ordering::Relaxed)
        }
        Kind::Tcp => sh.stats.via_tcp.fetch_add(1, Ordering::Relaxed),
        Kind::Warp => sh.stats.via_warp.fetch_add(1, Ordering::Relaxed),
    };
    match up {
        Upstream::Ws(io) => bridge_ws(s, io, ciphers, relay, Splitter::new(hello.proto), &sh.stats).await,
        Upstream::Tcp(t) => bridge_tcp(s, t, ciphers, relay, &sh.stats).await,
    }
}

async fn bridge_ws(client: TcpStream, up: Box<dyn route::Io>, c: Ciphers, relay: [u8; INIT_LEN], mut splitter: Splitter, stats: &Stats) {
    let Ciphers { mut clt_dec, mut clt_enc, mut tg_enc, mut tg_dec } = c;
    let (mut cr, mut cw) = client.into_split();
    let (ur, uw) = tokio::io::split(up);
    let uw = tokio::sync::Mutex::new(uw);
    if uw.lock().await.write_all(&ws::frame(ws::OP_BINARY, &relay)).await.is_err() {
        return;
    }
    let upstream = async {
        let mut buf = vec![0u8; BUF];
        loop {
            let n = match cr.read(&mut buf).await {
                Ok(0) | Err(_) => {
                    if let Some(tail) = splitter.flush() {
                        let _ = uw.lock().await.write_all(&ws::frame(ws::OP_BINARY, &tail)).await;
                    }
                    return;
                }
                Ok(n) => n,
            };
            stats.up.fetch_add(n as u64, Ordering::Relaxed);
            let mut plain = buf[..n].to_vec();
            obfs::apply(&mut clt_dec, &mut plain);
            let mut cipher = plain.clone();
            obfs::apply(&mut tg_enc, &mut cipher);
            let parts = splitter.split(&cipher, &plain);
            if parts.is_empty() {
                continue;
            }
            let out: Vec<u8> = parts.iter().flat_map(|p| ws::frame(ws::OP_BINARY, p)).collect();
            if uw.lock().await.write_all(&out).await.is_err() {
                return;
            }
        }
    };
    let downstream = async {
        let mut reader = ws::Reader::new(ur);
        loop {
            match reader.next().await {
                Ok(ws::Event::Data(mut d)) => {
                    stats.down.fetch_add(d.len() as u64, Ordering::Relaxed);
                    obfs::apply(&mut tg_dec, &mut d);
                    obfs::apply(&mut clt_enc, &mut d);
                    if cw.write_all(&d).await.is_err() {
                        return;
                    }
                }
                Ok(ws::Event::Ping(p)) => {
                    if uw.lock().await.write_all(&ws::frame(ws::OP_PONG, &p)).await.is_err() {
                        return;
                    }
                }
                Ok(ws::Event::Close(code)) => {
                    let _ = uw.lock().await.write_all(&ws::frame(ws::OP_CLOSE, &code)).await;
                    return;
                }
                Err(_) => return,
            }
        }
    };
    tokio::select! {
        _ = upstream => {}
        _ = downstream => {}
    }
    let mut w = uw.lock().await;
    let _ = w.write_all(&ws::frame(ws::OP_CLOSE, &[])).await;
    let _ = w.shutdown().await;
    let _ = cw.shutdown().await;
}

async fn bridge_tcp(client: TcpStream, up: TcpStream, c: Ciphers, relay: [u8; INIT_LEN], stats: &Stats) {
    let Ciphers { mut clt_dec, mut clt_enc, mut tg_enc, mut tg_dec } = c;
    let (mut cr, mut cw) = client.into_split();
    let (mut ur, mut uw) = up.into_split();
    if uw.write_all(&relay).await.is_err() {
        return;
    }
    let upstream = async {
        let mut buf = vec![0u8; BUF];
        while let Ok(n) = cr.read(&mut buf).await {
            if n == 0 {
                break;
            }
            stats.up.fetch_add(n as u64, Ordering::Relaxed);
            obfs::apply(&mut clt_dec, &mut buf[..n]);
            obfs::apply(&mut tg_enc, &mut buf[..n]);
            if uw.write_all(&buf[..n]).await.is_err() {
                break;
            }
        }
    };
    let downstream = async {
        let mut buf = vec![0u8; BUF];
        while let Ok(n) = ur.read(&mut buf).await {
            if n == 0 {
                break;
            }
            stats.down.fetch_add(n as u64, Ordering::Relaxed);
            obfs::apply(&mut tg_dec, &mut buf[..n]);
            obfs::apply(&mut clt_enc, &mut buf[..n]);
            if cw.write_all(&buf[..n]).await.is_err() {
                break;
            }
        }
    };
    tokio::select! {
        _ = upstream => {}
        _ = downstream => {}
    }
    let _ = uw.shutdown().await;
    let _ = cw.shutdown().await;
}

#[cfg(test)]
mod tests {
    use super::obfs::{Proto, tests as peers};
    use super::*;
    use ctr::cipher::StreamCipher;

    #[test]
    fn secret_and_link() {
        let s = [0xab; 16];
        let c = Config { port: 1443, secret: s, cloudflare: true, cf_domain: String::new() };
        assert_eq!(c.link(), format!("tg://proxy?server=127.0.0.1&port=1443&secret=dd{}", "ab".repeat(16)));
        assert_eq!(parse_secret(&hex(&s)), Some(s));
        assert_eq!(parse_secret(&format!("dd{}", hex(&s))), Some(s));
        assert_eq!(parse_secret("xyz"), None);
    }

    async fn fake_telegram(listener: TcpListener) -> Vec<u8> {
        let (mut s, _) = listener.accept().await.unwrap();
        let mut head = Vec::new();
        let mut b = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            s.read_exact(&mut b).await.unwrap();
            head.push(b[0]);
        }
        let head = String::from_utf8(head).unwrap();
        assert!(head.starts_with("GET /apiws HTTP/1.1\r\nHost: kws2.web.telegram.org\r\n"), "{head}");
        s.write_all(b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n\r\n").await.unwrap();
        let (r, mut w) = s.into_split();
        let mut reader = ws::Reader::new(r);
        let ws::Event::Data(relay) = reader.next().await.unwrap() else { panic!() };
        let relay: [u8; INIT_LEN] = relay.try_into().unwrap();
        let (proto, dc, mut peer) = peers::telegram(&relay);
        assert_eq!((proto, dc), (Proto::Intermediate, 2));
        w.write_all(&[0x89, 2, 1, 2]).await.unwrap();
        let mut got = Vec::new();
        while got.len() < 4 + 16 {
            match reader.next().await.unwrap() {
                ws::Event::Data(mut d) => {
                    peer.dec.apply_keystream(&mut d);
                    got.extend(d);
                }
                ws::Event::Ping(_) | ws::Event::Close(_) => panic!(),
            }
        }
        let mut reply = b"pong from DC2".to_vec();
        peer.enc.apply_keystream(&mut reply);
        let mut f = vec![0x82, reply.len() as u8];
        f.extend(reply);
        w.write_all(&f).await.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        got
    }

    #[test]
    fn end_to_end_through_websocket() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let tg = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let tg_addr = tg.local_addr().unwrap();
            let server = tokio::spawn(fake_telegram(tg));
            let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
            let secret = new_secret();
            let mut routes = Routes::new(None, false).unwrap();
            routes.plain_ws = Some(tg_addr);
            let cfg = Config { port, secret, cloudflare: false, cf_domain: String::new() };
            let handle = start_with(cfg, routes).unwrap();

            let (init, mut cl) = peers::client(&secret, Proto::Intermediate, 2);
            let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            s.write_all(&init).await.unwrap();
            let mut msg = 16u32.to_le_bytes().to_vec();
            msg.extend(b"0123456789abcdef");
            let mut wire = msg.clone();
            cl.enc.apply_keystream(&mut wire);
            s.write_all(&wire[..7]).await.unwrap();
            tokio::time::sleep(Duration::from_millis(50)).await;
            s.write_all(&wire[7..]).await.unwrap();

            let mut reply = vec![0u8; 13];
            timeout(Duration::from_secs(5), s.read_exact(&mut reply)).await.unwrap().unwrap();
            cl.dec.apply_keystream(&mut reply);
            assert_eq!(reply, b"pong from DC2");
            assert_eq!(server.await.unwrap(), msg);
            assert_eq!(handle.stats.via_ws.load(Ordering::Relaxed), 1);
            drop(handle);
        });
    }

    #[test]
    #[ignore]
    fn live_proxy_end_to_end() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
            let secret = new_secret();
            let handle = start(Config { port, secret, cloudflare: true, cf_domain: String::new() }).unwrap();
            for (dc, proto) in [(2i16, Proto::Secure), (-4, Proto::Intermediate), (1, Proto::Abridged), (5, Proto::Secure)] {
                let (init, mut cl) = peers::client(&secret, proto, dc);
                let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
                s.write_all(&init).await.unwrap();
                let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap();
                let mut p = vec![0u8; 8];
                p.extend(((now.as_secs() << 32) | (now.subsec_nanos() as u64 & 0xffff_fffc)).to_le_bytes());
                p.extend(20u32.to_le_bytes());
                p.extend(0xbe7e8ef1u32.to_le_bytes());
                p.extend(obfs::random::<16>());
                let mut pkt = match proto {
                    Proto::Abridged => vec![(p.len() / 4) as u8],
                    _ => (p.len() as u32).to_le_bytes().to_vec(),
                };
                pkt.extend(p);
                cl.enc.apply_keystream(&mut pkt);
                s.write_all(&pkt).await.unwrap();
                let mut head = vec![0u8; if proto == Proto::Abridged { 1 } else { 4 }];
                timeout(Duration::from_secs(30), s.read_exact(&mut head)).await.unwrap().unwrap();
                cl.dec.apply_keystream(&mut head);
                let len = if proto == Proto::Abridged { head[0] as usize * 4 } else { (u32::from_le_bytes(head[..4].try_into().unwrap()) & 0x7fffffff) as usize };
                let mut body = vec![0u8; len];
                s.read_exact(&mut body).await.unwrap();
                cl.dec.apply_keystream(&mut body);
                assert_eq!(body[20..24], 0x05162463u32.to_le_bytes(), "DC{dc}");
                println!("DC{dc} {proto:?}: resPQ через прокси");
            }
            let st = &handle.stats;
            println!(
                "ws={} cf={} tcp={} failed={}",
                st.via_ws.load(Ordering::Relaxed),
                st.via_cf.load(Ordering::Relaxed),
                st.via_tcp.load(Ordering::Relaxed),
                st.failed.load(Ordering::Relaxed)
            );
        });
    }

    #[test]
    fn wrong_secret_is_counted_and_dropped() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
            let cfg = Config { port, secret: new_secret(), cloudflare: false, cf_domain: String::new() };
            let handle = start(cfg).unwrap();
            let (init, _) = peers::client(&new_secret(), Proto::Secure, 2);
            let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            s.write_all(&init).await.unwrap();
            drop(s);
            for _ in 0..50 {
                if handle.stats.bad_secret.load(Ordering::Relaxed) == 1 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            assert_eq!(handle.stats.bad_secret.load(Ordering::Relaxed), 1);
        });
    }
}
