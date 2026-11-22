use super::obfs::random;
use super::ws;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::WebPkiServerVerifier;
use rustls::crypto::{CryptoProvider, ring};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{CertificateError, ClientConfig, DigitallySignedStruct, Error as TlsError, RootCertStore, SignatureScheme};
use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_rustls::TlsConnector;

pub trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

pub const FRONT_SNI: &str = "sprinthost.ru";
pub const WS_IP: Ipv4Addr = Ipv4Addr::new(149, 154, 167, 220);
const DC_IPS: [(u16, Ipv4Addr); 6] = [
    (1, Ipv4Addr::new(149, 154, 175, 50)),
    (2, Ipv4Addr::new(149, 154, 167, 51)),
    (3, Ipv4Addr::new(149, 154, 175, 100)),
    (4, Ipv4Addr::new(149, 154, 167, 91)),
    (5, Ipv4Addr::new(149, 154, 171, 5)),
    (203, Ipv4Addr::new(91, 105, 192, 100)),
];
const TEST_DC_IPS: [(u16, Ipv4Addr); 3] =
    [(1, Ipv4Addr::new(149, 154, 175, 10)), (2, Ipv4Addr::new(149, 154, 167, 40)), (3, Ipv4Addr::new(149, 154, 175, 117))];
pub const CF_DOMAINS: [&str; 20] = [
    "pclead.co.uk",
    "offshor.co.uk",
    "cakeisalie.co.uk",
    "noskomnadzor.co.uk",
    "lovetrue.co.uk",
    "sorokdva.co.uk",
    "pyatdesyatdva.co.uk",
    "kartoshka.co.uk",
    "sorokodin.co.uk",
    "pyatdesyatodin.co.uk",
    "notelega.co.uk",
    "ebally.co.uk",
    "nebally.co.uk",
    "havegreatday.co.uk",
    "pomogite.co.uk",
    "fixtelega.co.uk",
    "sadnews.co.uk",
    "onedaychamp.co.uk",
    "stopblocking.co.uk",
    "nothingthere.co.uk",
];
const WS_TIMEOUT: Duration = Duration::from_secs(8);
const FRONT_TIMEOUT: Duration = Duration::from_secs(7);
const CF_TIMEOUT: Duration = Duration::from_secs(10);
const TCP_TIMEOUT: Duration = Duration::from_secs(10);
const CF_TRIES: usize = 6;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Ws,
    Cf(String),
    Tcp,
}

pub enum Upstream {
    Ws(Box<dyn Io>),
    Tcp(TcpStream),
}

pub struct Target {
    pub dc: u16,
    pub media: bool,
    pub test: bool,
}

#[derive(Default)]
struct Backoff {
    fails: u32,
    until: Option<Instant>,
}

impl Backoff {
    fn blocked(&self) -> bool {
        self.until.is_some_and(|u| Instant::now() < u)
    }

    fn fail(&mut self, base: u64, cap: u64) {
        self.fails += 1;
        let secs = (base << (self.fails - 1).min(12)).min(cap);
        self.until = Some(Instant::now() + Duration::from_secs(secs));
    }
}

pub struct Routes {
    redirects: HashMap<u16, IpAddr>,
    cf_domains: Vec<String>,
    user_domain: Option<String>,
    normal: TlsConnector,
    fronted: TlsConnector,
    front_first: AtomicBool,
    ws_backoff: Mutex<HashMap<(u16, bool, bool), Backoff>>,
    cf_sticky: Mutex<HashMap<u16, String>>,
    tcp_backoff: Mutex<HashMap<IpAddr, Backoff>>,
    tcp_inflight: Mutex<HashSet<IpAddr>>,
    #[cfg(test)]
    pub plain_ws: Option<SocketAddr>,
}

#[derive(Debug)]
struct AnyName(Arc<WebPkiServerVerifier>);

impl ServerCertVerifier for AnyName {
    fn verify_server_cert(
        &self,
        end: &CertificateDer<'_>,
        inter: &[CertificateDer<'_>],
        name: &ServerName<'_>,
        ocsp: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, TlsError> {
        match self.0.verify_server_cert(end, inter, name, ocsp, now) {
            Err(TlsError::InvalidCertificate(CertificateError::NotValidForName))
            | Err(TlsError::InvalidCertificate(CertificateError::NotValidForNameContext { .. })) => Ok(ServerCertVerified::assertion()),
            other => other,
        }
    }

    fn verify_tls12_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, TlsError> {
        self.0.verify_tls12_signature(m, c, d)
    }

    fn verify_tls13_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, TlsError> {
        self.0.verify_tls13_signature(m, c, d)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.supported_verify_schemes()
    }
}

fn provider() -> Arc<CryptoProvider> {
    let mut p = ring::default_provider();
    p.kx_groups = vec![ring::kx_group::X25519];
    Arc::new(p)
}

fn connectors() -> anyhow::Result<(TlsConnector, TlsConnector)> {
    let roots = Arc::new(RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() });
    let p = provider();
    let normal = ClientConfig::builder_with_provider(p.clone())
        .with_safe_default_protocol_versions()?
        .with_root_certificates(roots.clone())
        .with_no_client_auth();
    let verifier = WebPkiServerVerifier::builder_with_provider(roots, p.clone()).build()?;
    let fronted = ClientConfig::builder_with_provider(p)
        .with_safe_default_protocol_versions()?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AnyName(verifier)))
        .with_no_client_auth();
    Ok((TlsConnector::from(Arc::new(normal)), TlsConnector::from(Arc::new(fronted))))
}

pub fn ws_domains(dc: u16, media: bool) -> Vec<String> {
    let dc = if dc == 203 { 2 } else { dc };
    if media {
        vec![format!("kws{dc}-1.web.telegram.org"), format!("kws{dc}.web.telegram.org")]
    } else {
        vec![format!("kws{dc}.web.telegram.org")]
    }
}

pub fn dc_ip(dc: u16, test: bool) -> Option<IpAddr> {
    let table: &[(u16, Ipv4Addr)] = if test { &TEST_DC_IPS } else { &DC_IPS };
    table.iter().find(|(d, _)| *d == dc).map(|(_, ip)| IpAddr::V4(*ip))
}

pub fn normalize_domain(raw: &str) -> Option<String> {
    let d = raw.trim().trim_start_matches("https://").trim_start_matches("http://").trim_end_matches('/').to_lowercase();
    let ok = d.contains('.')
        && d.split('.').all(|l| !l.is_empty() && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
    ok.then_some(d)
}

fn shuffled(mut v: Vec<String>) -> Vec<String> {
    for i in (1..v.len()).rev() {
        let r = u32::from_le_bytes(random::<4>()) as usize % (i + 1);
        v.swap(i, r);
    }
    v
}

impl Routes {
    pub fn new(user_domain: Option<&str>, cf: bool) -> anyhow::Result<Routes> {
        let (normal, fronted) = connectors()?;
        let user_domain = user_domain.and_then(normalize_domain).filter(|_| cf);
        let mut cf_domains: Vec<String> = user_domain.iter().cloned().collect();
        if cf {
            for d in CF_DOMAINS {
                if !cf_domains.iter().any(|x| x == d) {
                    cf_domains.push(d.to_string());
                }
            }
        }
        let redirects = [(2u16, IpAddr::V4(WS_IP)), (4, IpAddr::V4(WS_IP))].into_iter().collect();
        Ok(Routes {
            redirects,
            cf_domains,
            user_domain,
            normal,
            fronted,
            front_first: AtomicBool::new(false),
            ws_backoff: Mutex::default(),
            cf_sticky: Mutex::default(),
            tcp_backoff: Mutex::default(),
            tcp_inflight: Mutex::default(),
            #[cfg(test)]
            plain_ws: None,
        })
    }

    async fn ws_over<S: Io + 'static>(&self, mut s: S, host: &str, path: &str) -> anyhow::Result<Box<dyn Io>> {
        ws::handshake(&mut s, host, path).await.map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(Box::new(s))
    }

    async fn wss(&self, addr: SocketAddr, sni: &str, host: &str, path: &str, fronted: bool) -> anyhow::Result<Box<dyn Io>> {
        let tcp = TcpStream::connect(addr).await?;
        tcp.set_nodelay(true)?;
        let name = ServerName::try_from(sni.to_string())?;
        let conn = if fronted { &self.fronted } else { &self.normal };
        let tls = conn.connect(name, tcp).await?;
        self.ws_over(tls, host, path).await
    }

    async fn direct(&self, t: &Target) -> Option<Box<dyn Io>> {
        let ip = *self.redirects.get(&t.dc)?;
        let key = (t.dc, t.media, t.test);
        if self.ws_backoff.lock().unwrap().get(&key).is_some_and(Backoff::blocked) {
            return None;
        }
        let path = if t.test { "/apiws_test" } else { "/apiws" };
        #[cfg(test)]
        if let Some(addr) = self.plain_ws {
            let tcp = TcpStream::connect(addr).await.ok()?;
            return self.ws_over(tcp, &ws_domains(t.dc, t.media)[0], path).await.ok();
        }
        let first = self.front_first.load(Ordering::Relaxed);
        for host in ws_domains(t.dc, t.media) {
            for fronted in [first, !first] {
                let (sni, limit) = if fronted { (FRONT_SNI, FRONT_TIMEOUT) } else { (host.as_str(), WS_TIMEOUT) };
                let r = timeout(limit, self.wss(SocketAddr::new(ip, 443), sni, &host, path, fronted)).await;
                if let Ok(Ok(s)) = r {
                    self.front_first.store(fronted, Ordering::Relaxed);
                    self.ws_backoff.lock().unwrap().remove(&key);
                    return Some(s);
                }
            }
        }
        self.ws_backoff.lock().unwrap().entry(key).or_default().fail(1, 3600);
        None
    }

    fn cf_order(&self, dc: u16) -> Vec<String> {
        let sticky = self.cf_sticky.lock().unwrap().get(&dc).cloned();
        let mut v: Vec<String> = sticky.or_else(|| self.user_domain.clone()).into_iter().collect();
        let rest = self.cf_domains.iter().filter(|d| !v.contains(d)).cloned().collect();
        v.extend(shuffled(rest));
        v
    }

    async fn cloudflare(&self, t: &Target) -> Option<(String, Box<dyn Io>)> {
        if t.test || self.cf_domains.is_empty() {
            return None;
        }
        for base in self.cf_order(t.dc).into_iter().take(CF_TRIES) {
            let host = format!("kws{}.{base}", t.dc);
            let attempt = async {
                let addr = tokio::net::lookup_host((host.as_str(), 443)).await?.next().ok_or_else(|| anyhow::anyhow!("нет адреса"))?;
                self.wss(addr, &host, &host, "/apiws", false).await
            };
            if let Ok(Ok(s)) = timeout(CF_TIMEOUT, attempt).await {
                self.cf_sticky.lock().unwrap().insert(t.dc, base.clone());
                return Some((base, s));
            }
        }
        None
    }

    async fn tcp(&self, t: &Target) -> Option<TcpStream> {
        let ip = dc_ip(t.dc, t.test)?;
        if self.tcp_backoff.lock().unwrap().get(&ip).is_some_and(Backoff::blocked) {
            return None;
        }
        if !self.tcp_inflight.lock().unwrap().insert(ip) {
            return None;
        }
        let r = timeout(TCP_TIMEOUT, TcpStream::connect(SocketAddr::new(ip, 443))).await;
        self.tcp_inflight.lock().unwrap().remove(&ip);
        match r {
            Ok(Ok(s)) => {
                let _ = s.set_nodelay(true);
                self.tcp_backoff.lock().unwrap().remove(&ip);
                Some(s)
            }
            _ => {
                self.tcp_backoff.lock().unwrap().entry(ip).or_default().fail(30, 3600);
                None
            }
        }
    }

    pub async fn connect(&self, t: &Target) -> Option<(Kind, Upstream)> {
        if let Some(s) = self.direct(t).await {
            return Some((Kind::Ws, Upstream::Ws(s)));
        }
        if let Some((d, s)) = self.cloudflare(t).await {
            return Some((Kind::Cf(d), Upstream::Ws(s)));
        }
        self.tcp(t).await.map(|s| (Kind::Tcp, Upstream::Tcp(s)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use super::super::obfs::{self, Proto};
    use ctr::cipher::StreamCipher;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn req_pq() -> Vec<u8> {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap();
        let msg_id = (now.as_secs() << 32) | ((now.subsec_nanos() as u64) << 2 & 0xffff_fffc);
        let mut p = vec![0u8; 8];
        p.extend(msg_id.to_le_bytes());
        p.extend(20u32.to_le_bytes());
        p.extend(0xbe7e8ef1u32.to_le_bytes());
        p.extend(obfs::random::<16>());
        let mut out = (p.len() as u32).to_le_bytes().to_vec();
        out.extend(p);
        out
    }

    fn is_res_pq(plain: &[u8]) -> bool {
        plain.len() >= 28 && plain[24..28] == 0x05162463u32.to_le_bytes()
    }

    async fn ask(up: Upstream, dc: u16) -> bool {
        let relay = obfs::relay_init(Proto::Intermediate, dc as i16);
        let (_, _, peer) = obfs::tests::telegram(&relay);
        let (mut enc, mut dec) = (peer.dec, peer.enc);
        let mut pkt = req_pq();
        enc.apply_keystream(&mut pkt);
        match up {
            Upstream::Ws(mut s) => {
                s.write_all(&ws::frame(ws::OP_BINARY, &relay)).await.unwrap();
                s.write_all(&ws::frame(ws::OP_BINARY, &pkt)).await.unwrap();
                let mut r = ws::Reader::new(s);
                loop {
                    if let ws::Event::Data(mut d) = r.next().await.unwrap() {
                        dec.apply_keystream(&mut d);
                        return is_res_pq(&d);
                    }
                }
            }
            Upstream::Tcp(mut s) => {
                s.write_all(&relay).await.unwrap();
                s.write_all(&pkt).await.unwrap();
                let mut d = vec![0u8; 88];
                s.read_exact(&mut d).await.unwrap();
                dec.apply_keystream(&mut d);
                is_res_pq(&d)
            }
        }
    }

    #[tokio::test(flavor = "current_thread")]
    #[ignore]
    async fn live_direct_ws() {
        let r = Routes::new(None, false).unwrap();
        for (dc, media) in [(2u16, false), (4, false), (2, true)] {
            let s = r.direct(&Target { dc, media, test: false }).await.expect("direct ws");
            assert!(ask(Upstream::Ws(s), dc).await, "DC{dc}");
            println!("direct DC{dc} media={media}: resPQ, fronting={}", r.front_first.load(Ordering::Relaxed));
        }
        let f = Routes::new(None, false).unwrap();
        f.front_first.store(true, Ordering::Relaxed);
        let s = f.direct(&Target { dc: 2, media: false, test: false }).await.expect("fronted ws");
        assert!(ask(Upstream::Ws(s), 2).await);
        println!("fronted DC2: resPQ, fronting kept={}", f.front_first.load(Ordering::Relaxed));
    }

    #[tokio::test(flavor = "current_thread")]
    #[ignore]
    async fn live_cloudflare() {
        let r = Routes::new(None, true).unwrap();
        for dc in [1u16, 3, 5] {
            let (d, s) = r.cloudflare(&Target { dc, media: false, test: false }).await.expect("cf");
            assert!(ask(Upstream::Ws(s), dc).await, "DC{dc}");
            println!("cf DC{dc} via {d}: resPQ");
        }
    }

    #[tokio::test(flavor = "current_thread")]
    #[ignore]
    async fn live_tcp() {
        let r = Routes::new(None, false).unwrap();
        let s = r.tcp(&Target { dc: 2, media: false, test: false }).await.expect("tcp");
        assert!(ask(Upstream::Tcp(s), 2).await);
        println!("tcp DC2: resPQ");
    }

    #[test]
    fn domains_follow_reference_rules() {
        assert_eq!(ws_domains(2, false), vec!["kws2.web.telegram.org"]);
        assert_eq!(ws_domains(4, true), vec!["kws4-1.web.telegram.org", "kws4.web.telegram.org"]);
        assert_eq!(ws_domains(203, false), vec!["kws2.web.telegram.org"]);
        assert_eq!(dc_ip(203, false), Some("91.105.192.100".parse().unwrap()));
        assert_eq!(dc_ip(2, true), Some("149.154.167.40".parse().unwrap()));
        assert_eq!(dc_ip(5, true), None);
        assert_eq!(normalize_domain(" https://Example.com/ "), Some("example.com".into()));
        assert_eq!(normalize_domain("bad"), None);
    }

    #[test]
    fn backoff_grows_and_caps() {
        let mut b = Backoff::default();
        b.fail(30, 3600);
        assert!(b.blocked());
        for _ in 0..20 {
            b.fail(30, 3600);
        }
        let left = b.until.unwrap() - Instant::now();
        assert!(left <= Duration::from_secs(3600) && left > Duration::from_secs(3500));
    }

    #[test]
    fn user_domain_goes_first() {
        let r = Routes::new(Some("my.example.org"), true).unwrap();
        assert_eq!(r.cf_order(2)[0], "my.example.org");
        assert_eq!(r.cf_order(2).len(), 21);
        r.cf_sticky.lock().unwrap().insert(2, "kartoshka.co.uk".into());
        assert_eq!(r.cf_order(2)[0], "kartoshka.co.uk");
        assert!(Routes::new(None, false).unwrap().cf_order(2).is_empty());
        let firsts: HashSet<String> = (0..40).map(|_| Routes::new(None, true).unwrap().cf_order(3)[0].clone()).collect();
        assert!(firsts.len() > 1);
    }
}
