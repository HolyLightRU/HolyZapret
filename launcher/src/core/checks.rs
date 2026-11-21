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

pub fn check_one(site: &Site) -> SiteResult {
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
