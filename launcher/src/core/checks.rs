use super::lists::normalize_domain;
use super::paths::Paths;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;
use std::sync::mpsc::Sender;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
pub const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36";

#[derive(Clone, Debug)]
pub struct Site {
    pub url: String,
    pub custom: bool,
}

#[derive(Clone, Debug)]
pub struct SiteResult {
    pub url: String,
    pub ok: bool,
    pub code: u16,
    pub detail: String,
    pub ms: u128,
    pub custom: bool,
}

impl SiteResult {
    pub fn warn(&self) -> bool {
        self.ok && self.code >= 400
    }
}

pub fn my_sites_path(paths: &Paths) -> PathBuf {
    paths.lists().join("my-sites.txt")
}

fn read_urls(path: &PathBuf) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("http"))
        .map(String::from)
        .collect()
}

pub fn load_my_sites(paths: &Paths) -> Vec<String> {
    read_urls(&my_sites_path(paths))
}

pub fn load_sites(paths: &Paths) -> Vec<Site> {
    let mut out: Vec<Site> = read_urls(&paths.check_list()).into_iter().map(|url| Site { url, custom: false }).collect();
    for url in load_my_sites(paths) {
        if !out.iter().any(|s| s.url.eq_ignore_ascii_case(&url)) {
            out.push(Site { url, custom: true });
        }
    }
    out
}

pub fn normalize_url(raw: &str) -> Result<String, String> {
    let t = raw.trim();
    let with_scheme = if t.contains("://") { t.to_string() } else { format!("https://{t}") };
    if !(with_scheme.starts_with("https://") || with_scheme.starts_with("http://")) {
        return Err(format!("{raw}: нужна ссылка http или https"));
    }
    let host = normalize_domain(&with_scheme)?;
    let rest = with_scheme.split_once("://").map(|x| x.1).unwrap_or("");
    let path = rest.find('/').map(|i| &rest[i..]).unwrap_or("/");
    let scheme = with_scheme.split("://").next().unwrap_or("https").to_lowercase();
    Ok(format!("{scheme}://{host}{path}"))
}

fn write_my_sites(paths: &Paths, urls: &[String]) -> anyhow::Result<()> {
    let mut text = urls.join("\r\n");
    if !text.is_empty() {
        text.push_str("\r\n");
    }
    std::fs::create_dir_all(paths.lists())?;
    std::fs::write(my_sites_path(paths), text)?;
    Ok(())
}

pub fn add_my_site(paths: &Paths, raw: &str) -> anyhow::Result<String> {
    let url = normalize_url(raw).map_err(|e| anyhow::anyhow!(e))?;
    let mut urls = load_my_sites(paths);
    let known = load_sites(paths).iter().any(|s| s.url.eq_ignore_ascii_case(&url));
    if known {
        anyhow::bail!("{url} уже есть в списке");
    }
    urls.push(url.clone());
    write_my_sites(paths, &urls)?;
    Ok(url)
}

pub fn remove_my_site(paths: &Paths, url: &str) -> anyhow::Result<()> {
    let urls: Vec<String> = load_my_sites(paths).into_iter().filter(|u| !u.eq_ignore_ascii_case(url)).collect();
    write_my_sites(paths, &urls)
}

pub fn curl_error(exit: i32) -> String {
    match exit {
        6 => "DNS не найден".into(),
        7 => "соединение отклонено".into(),
        28 => "таймаут".into(),
        35 | 60 => "ошибка TLS".into(),
        52 | 56 => "соединение сброшено".into(),
        -1 => "не удалось собрать запрос".into(),
        n => format!("ошибка curl {n}"),
    }
}

pub fn judge(url: &str, custom: bool, exit: Option<i32>, out: &str) -> SiteResult {
    let mut it = out.split_whitespace();
    let code: u16 = it.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    let secs: f64 = it.next().and_then(|v| v.replace(',', ".").parse().ok()).unwrap_or(0.0);
    let ok = code > 0;
    let detail = if ok { format!("HTTP {code}") } else { curl_error(exit.unwrap_or(-1)) };
    SiteResult { url: url.to_string(), ok, code, detail, ms: (secs * 1000.0) as u128, custom }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hello {
    Answered,
    Refused,
}

pub fn tls13_hello(host: &str) -> Option<Vec<u8>> {
    let mut h = rayoflight::tls::with_sni(rayoflight::desync::FAKE_DEFAULT, host)?;
    rayoflight::sys::random(&mut h[11..43]);
    let sid = h[43] as usize;
    rayoflight::sys::random(&mut h[44..44 + sid]);
    Some(h)
}

pub fn tls13_probe(host: &str, timeout: std::time::Duration) -> (Result<Hello, i32>, u128) {
    use std::io::{Read, Write};
    use std::net::{TcpStream, ToSocketAddrs};
    let start = std::time::Instant::now();
    let ms = || start.elapsed().as_millis();
    let Some(addr) = (host, 443).to_socket_addrs().ok().and_then(|mut a| a.next()) else { return (Err(6), ms()) };
    let Some(hello) = tls13_hello(host) else { return (Err(-1), ms()) };
    let mut s = match TcpStream::connect_timeout(&addr, timeout) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::TimedOut => return (Err(28), ms()),
        Err(_) => return (Err(7), ms()),
    };
    let _ = s.set_read_timeout(Some(timeout.saturating_sub(start.elapsed()).max(std::time::Duration::from_millis(200))));
    if s.write_all(&hello).is_err() {
        return (Err(56), ms());
    }
    let mut buf = [0u8; 16];
    let mut got = 0;
    while got < 6 {
        match s.read(&mut buf[got..]) {
            Ok(0) => return (Err(52), ms()),
            Ok(n) => got += n,
            Err(e) if matches!(e.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock) => return (Err(28), ms()),
            Err(_) => return (Err(56), ms()),
        }
    }
    let r = match (buf[0], buf[5]) {
        (0x16, 0x02) => Ok(Hello::Answered),
        (0x15, _) => Ok(Hello::Refused),
        _ => Err(35),
    };
    (r, ms())
}

fn probe_site(site: &Site) -> SiteResult {
    let host = super::picker::host_of(&site.url);
    let (r, ms) = tls13_probe(&host, std::time::Duration::from_secs(6));
    let (ok, detail) = match r {
        Ok(Hello::Answered) => (true, "TLS 1.3".to_string()),
        Ok(Hello::Refused) => (true, "TLS 1.3, отказ".to_string()),
        Err(e) => (false, curl_error(e)),
    };
    SiteResult { url: site.url.clone(), ok, code: 0, detail, ms, custom: site.custom }
}

pub fn check_one(site: &Site) -> SiteResult {
    if site.url.starts_with("https://") && !super::picker::tls13_supported() {
        return probe_site(site);
    }
    let out = Command::new("curl.exe")
        .args(["-s", "-o", "NUL", "-L", "-m", "6", "-A", UA, "-w", "%{http_code} %{time_total}", &site.url])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    match out {
        Ok(o) => judge(&site.url, site.custom, o.status.code(), &String::from_utf8_lossy(&o.stdout)),
        Err(_) => SiteResult {
            url: site.url.clone(),
            ok: false,
            code: 0,
            detail: "curl.exe не найден".into(),
            ms: 0,
            custom: site.custom,
        },
    }
}

pub fn check_all(sites: Vec<Site>, tx: Sender<SiteResult>) {
    for site in sites {
        let tx = tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(check_one(&site));
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tls13_hello_names_the_site() {
        let h = tls13_hello("www.youtube.com").unwrap();
        assert_eq!(rayoflight::tls::sni(&h).as_deref(), Some("www.youtube.com"));
        assert_eq!(rayoflight::tls::record_len(&h), Some(h.len()));
        let again = tls13_hello("www.youtube.com").unwrap();
        assert_ne!(h[11..43], again[11..43]);
        assert_eq!(curl_error(-1), "не удалось собрать запрос");
    }

    #[test]
    #[ignore]
    fn tls13_probe_live() {
        for host in ["discord.com", "www.google.com", "www.youtube.com", "rutracker.org"] {
            let (r, ms) = tls13_probe(host, std::time::Duration::from_secs(6));
            assert_eq!(r, Ok(Hello::Answered), "{host} {ms}");
        }
        assert_eq!(tls13_probe("no-such-host.invalid", std::time::Duration::from_secs(3)).0, Err(6));
    }

    #[test]
    fn responses_judged() {
        let ok = judge("https://a.com", false, Some(0), "200 0.150");
        assert!(ok.ok && !ok.warn() && ok.ms == 150);
        let limited = judge("https://a.com", false, Some(0), "429 0,581");
        assert!(limited.ok && limited.warn() && limited.detail == "HTTP 429");
        let slow = judge("https://a.com", false, Some(28), "200 6.0");
        assert!(slow.ok);
        let dead = judge("https://a.com", true, Some(28), "000 6.0");
        assert!(!dead.ok && dead.detail == "таймаут" && dead.custom);
        assert_eq!(judge("x", false, Some(6), "000 0").detail, "DNS не найден");
    }

    #[test]
    fn urls_normalized() {
        assert_eq!(normalize_url("Open.Spotify.com").unwrap(), "https://open.spotify.com/");
        assert_eq!(normalize_url("https://x.com/home?a=1").unwrap(), "https://x.com/home?a=1");
        assert_eq!(normalize_url("http://Example.org").unwrap(), "http://example.org/");
        assert!(normalize_url("ftp://x.com").is_err());
        assert!(normalize_url("localhost").is_err());
    }
}
