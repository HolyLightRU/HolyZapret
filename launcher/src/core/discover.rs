use std::collections::{BTreeMap, BTreeSet};
use std::net::{IpAddr, ToSocketAddrs};
use std::sync::mpsc::channel;
use std::time::Duration;
use ureq::ResponseExt;

const MULTI_SUFFIXES: &[&str] = &[
    "co.uk", "org.uk", "ac.uk", "gov.uk", "com.ru", "net.ru", "org.ru", "pp.ru", "msk.ru", "spb.ru", "com.ua",
    "org.ua", "net.ua", "pp.ua", "kiev.ua", "co.jp", "ne.jp", "com.br", "com.au", "net.au", "com.tr", "co.il",
    "co.kr", "com.cn", "com.tw", "com.hk", "co.in", "github.io", "gitlab.io", "herokuapp.com", "vercel.app",
    "netlify.app", "pages.dev", "workers.dev", "appspot.com", "blogspot.com", "cloudfront.net", "amazonaws.com",
    "azureedge.net", "web.app", "firebaseapp.com",
];

const THIRD_PARTY: &[&str] = &[
    "google-analytics.com", "googletagmanager.com", "doubleclick.net", "googlesyndication.com",
    "googleadservices.com", "cdnjs.cloudflare.com", "cloudflareinsights.com", "jsdelivr.net", "unpkg.com",
    "yandex.ru", "yandex.net", "yastatic.net", "mc.yandex.ru", "facebook.net", "facebook.com", "hotjar.com",
    "sentry.io", "sentry-cdn.com", "recaptcha.net", "gstatic.com", "googleapis.com", "vk.com", "mail.ru",
    "top-fwz1.mail.ru", "twitter.com", "x.com", "linkedin.com", "bing.com", "clarity.ms", "newrelic.com",
    "nr-data.net", "segment.io", "segment.com", "intercom.io", "onesignal.com", "jquery.com", "bootstrapcdn.com",
];

const CLOUDFLARE_V4: &[(u32, u32)] = &[
    (0xADF5_3000, 20),
    (0x6715_F400, 22),
    (0x6716_C800, 22),
    (0x671F_0400, 22),
    (0x8D65_4000, 18),
    (0x6CA2_C000, 18),
    (0xBE5D_F000, 20),
    (0xBC72_6000, 20),
    (0xC5EA_F000, 22),
    (0xC629_8000, 17),
    (0xA29E_0000, 15),
    (0x6810_0000, 13),
    (0x6818_0000, 14),
    (0xAC40_0000, 13),
    (0x8300_4800, 22),
];

pub fn is_cloudflare(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let x = u32::from(*v4);
            CLOUDFLARE_V4.iter().any(|(net, p)| x & (u32::MAX << (32 - p)) == *net)
        }
        IpAddr::V6(v6) => {
            let s = v6.segments();
            matches!((s[0], s[1]), (0x2400, 0xcb00) | (0x2606, 0x4700) | (0x2803, 0xf800) | (0x2405, 0xb500) | (0x2405, 0x8100) | (0x2a06, 0x98c0) | (0x2c0f, 0xf248))
        }
    }
}

const NOISE: &[&str] = &["w3.org", "schema.org", "ogp.me", "purl.org", "xmlns.com", "example.com", "example.org"];

const FILE_TLDS: &[&str] = &[
    "js", "css", "png", "jpg", "jpeg", "svg", "json", "html", "htm", "php", "map", "gif", "webp", "woff", "woff2",
    "ttf", "ico", "txt", "xml", "mp4", "webm", "avif", "min", "id", "prototype", "length", "push", "call",
];

#[derive(Clone, Debug)]
pub struct Group {
    pub base: String,
    pub hosts: Vec<String>,
    pub ips: Vec<IpAddr>,
    pub main: bool,
    pub related: bool,
    pub third_party: bool,
    pub cloudflare: bool,
}

#[derive(Clone, Debug)]
pub struct Found {
    pub final_url: String,
    pub groups: Vec<Group>,
    pub assets_scanned: usize,
}

pub fn base_domain(host: &str) -> String {
    let parts: Vec<&str> = host.split('.').collect();
    if parts.len() <= 2 {
        return host.to_string();
    }
    let last2 = parts[parts.len() - 2..].join(".");
    let n = if MULTI_SUFFIXES.contains(&last2.as_str()) { 3 } else { 2 };
    parts[parts.len() - n.min(parts.len())..].join(".")
}

fn valid_host(h: &str) -> bool {
    let labels: Vec<&str> = h.split('.').collect();
    if labels.len() < 2 || h.len() > 253 {
        return false;
    }
    let tld = labels[labels.len() - 1];
    if tld.len() < 2 || tld.len() > 24 || !tld.chars().all(|c| c.is_ascii_alphabetic()) || FILE_TLDS.contains(&tld) {
        return false;
    }
    labels.iter().all(|l| !l.is_empty() && !l.starts_with('-') && !l.ends_with('-'))
}

fn is_host_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '.' || c == '-'
}

pub fn extract_hosts(text: &str) -> BTreeSet<String> {
    let text = text.replace("\\/", "/").replace("\\u002F", "/").replace("&#x2F;", "/");
    let mut out = BTreeSet::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while let Some(pos) = text[i..].find("//") {
        let start = i + pos + 2;
        let prev = if i + pos > 0 { bytes[i + pos - 1] as char } else { ' ' };
        let ok_prefix = prev == ':' || prev == '"' || prev == '\'' || prev == '(' || prev == '=' || prev == ' ';
        if ok_prefix {
            let host: String = text[start..].chars().take_while(|c| is_host_char(*c)).collect();
            let host = host.trim_matches(['.', '-']).to_lowercase();
            if valid_host(&host) && !NOISE.contains(&base_domain(&host).as_str()) {
                out.insert(host);
            }
        }
        i = start;
    }
    out
}

fn asset_urls(html: &str, origin: &str) -> Vec<String> {
    let mut out = Vec::new();
    for pat in ["src=\"", "href=\"", "src='", "href='"] {
        let quote = if pat.ends_with('"') { '"' } else { '\'' };
        let mut rest = html;
        while let Some(p) = rest.find(pat) {
            rest = &rest[p + pat.len()..];
            let val: String = rest.chars().take_while(|c| *c != quote).collect();
            let path = val.split(['?', '#']).next().unwrap_or("").to_lowercase();
            if !(path.ends_with(".js") || path.ends_with(".css")) {
                continue;
            }
            let url = if val.starts_with("http://") || val.starts_with("https://") {
                val
            } else if let Some(r) = val.strip_prefix("//") {
                format!("https://{r}")
            } else if val.starts_with('/') {
                format!("{origin}{val}")
            } else {
                continue;
            };
            if !out.contains(&url) {
                out.push(url);
            }
        }
    }
    out
}

fn related(brand: &str, base: &str) -> bool {
    let label = base.split('.').next().unwrap_or("");
    brand.len() >= 4 && label.len() >= 4 && (label.contains(brand) || brand.contains(label))
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36")
        .http_status_as_error(false)
        .build()
        .into()
}

fn host_of(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let h: String = rest.chars().take_while(|c| is_host_char(*c)).collect();
    Some(h.to_lowercase())
}

fn resolve(hosts: Vec<String>) -> BTreeMap<String, Vec<IpAddr>> {
    let (tx, rx) = channel();
    let n = hosts.len();
    for h in hosts {
        let tx = tx.clone();
        std::thread::spawn(move || {
            let mut ips: Vec<IpAddr> = (h.as_str(), 443)
                .to_socket_addrs()
                .map(|it| it.map(|a| a.ip()).collect())
                .unwrap_or_default();
            ips.sort();
            ips.dedup();
            let _ = tx.send((h, ips));
        });
    }
    drop(tx);
    rx.iter().take(n).collect()
}

pub fn run(input: &str) -> anyhow::Result<Found> {
    let mut url = input.trim().to_string();
    if !url.contains("://") {
        url = format!("https://{url}");
    }
    let start_host = host_of(&url).filter(|h| valid_host(h)).ok_or_else(|| anyhow::anyhow!("не похоже на адрес сайта"))?;
    let agent = agent();
    let mut resp = agent.get(&url).call()?;
    let final_url = resp.get_uri().to_string();
    let html = resp.body_mut().read_to_string().unwrap_or_default();
    let final_host = host_of(&final_url).unwrap_or_else(|| start_host.clone());
    let origin = format!("https://{final_host}");
    let main_base = base_domain(&final_host);
    let brand = main_base.split('.').next().unwrap_or("").to_string();

    let mut hosts = extract_hosts(&html);
    hosts.insert(start_host.clone());
    hosts.insert(final_host.clone());

    let assets: Vec<String> = asset_urls(&html, &origin)
        .into_iter()
        .filter(|u| host_of(u).is_some_and(|h| !THIRD_PARTY.iter().any(|t| base_domain(&h) == *t || h == *t)))
        .take(12)
        .collect();
    let scanned = assets.len();
    let (tx, rx) = channel();
    for a in assets {
        let tx = tx.clone();
        let agent = agent.clone();
        std::thread::spawn(move || {
            let text = agent
                .get(&a)
                .call()
                .ok()
                .and_then(|mut r| r.body_mut().with_config().limit(8 * 1024 * 1024).read_to_string().ok())
                .unwrap_or_default();
            let _ = tx.send(extract_hosts(&text));
        });
    }
    drop(tx);
    for set in rx.iter().take(scanned) {
        hosts.extend(set);
    }

    let hosts: Vec<String> = hosts.into_iter().take(80).collect();
    let resolved = resolve(hosts.clone());

    let mut groups: BTreeMap<String, Group> = BTreeMap::new();
    for h in hosts {
        let ips = resolved.get(&h).cloned().unwrap_or_default();
        if ips.is_empty() && h != final_host {
            continue;
        }
        let base = base_domain(&h);
        let g = groups.entry(base.clone()).or_insert_with(|| Group {
            third_party: THIRD_PARTY.contains(&base.as_str()),
            main: base == main_base,
            related: false,
            base: base.clone(),
            hosts: Vec::new(),
            ips: Vec::new(),
            cloudflare: false,
        });
        if THIRD_PARTY.contains(&h.as_str()) {
            g.third_party = true;
        }
        g.hosts.push(h);
        for ip in ips {
            if !g.ips.contains(&ip) {
                g.ips.push(ip);
            }
        }
    }
    let mut groups: Vec<Group> = groups
        .into_values()
        .map(|mut g| {
            g.cloudflare = g.ips.iter().any(is_cloudflare);
            if g.main {
                g.third_party = false;
            }
            g.related = !g.main && !g.third_party && related(&brand, &g.base);
            g
        })
        .collect();
    groups.sort_by_key(|g| (!g.main, !g.related, g.third_party, g.base.clone()));
    Ok(Found { final_url, groups, assets_scanned: scanned })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloudflare_ranges() {
        assert!(is_cloudflare(&"104.16.1.1".parse().unwrap()));
        assert!(is_cloudflare(&"172.67.10.10".parse().unwrap()));
        assert!(is_cloudflare(&"2606:4700::6810:84e5".parse().unwrap()));
        assert!(!is_cloudflare(&"142.250.75.174".parse().unwrap()));
        assert!(!is_cloudflare(&"8.8.8.8".parse().unwrap()));
    }

    #[test]
    fn related_brands() {
        assert!(related("spotify", "spotifycdn.com"));
        assert!(related("discord", "discordapp.com"));
        assert!(!related("spotify", "scdn.co"));
        assert!(!related("vk", "vkuser.net"));
    }

    #[test]
    fn base_domains() {
        assert_eq!(base_domain("open.spotify.com"), "spotify.com");
        assert_eq!(base_domain("spotify.com"), "spotify.com");
        assert_eq!(base_domain("www.bbc.co.uk"), "bbc.co.uk");
        assert_eq!(base_domain("d1234.cloudfront.net"), "d1234.cloudfront.net");
    }

    #[test]
    fn hosts_from_html() {
        let html = r#"<script src="https://cdn.example-site.com/app.js"></script>
            <link href="//fonts.site-cdn.net/a.css"> <a href="/local">x</a>
            <svg xmlns="http://www.w3.org/2000/svg"></svg>
            {"api":"https:\/\/api.spotify.com\/v1"} // jquery.min.js comment
            <img src=https://img.test-cdn.org/x.png>"#;
        let h = extract_hosts(html);
        assert!(h.contains("cdn.example-site.com"));
        assert!(h.contains("fonts.site-cdn.net"));
        assert!(h.contains("api.spotify.com"));
        assert!(h.contains("img.test-cdn.org"));
        assert!(!h.iter().any(|x| x.contains("w3.org")));
        assert!(!h.iter().any(|x| x.contains("jquery")));
    }

    #[test]
    fn assets_resolved_against_origin() {
        let html = r#"<script src="/static/main.js?v=1"></script><link href="https://x.com/s.css"><img src="/a.png">"#;
        let a = asset_urls(html, "https://site.com");
        assert_eq!(a, vec!["https://site.com/static/main.js?v=1".to_string(), "https://x.com/s.css".to_string()]);
    }
}

