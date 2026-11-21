use std::fs;
use std::net::IpAddr;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Domains,
    Ips,
}

pub const DOMAIN_PLACEHOLDER: &str = "domain.example.abc";
pub const IP_PLACEHOLDER: &str = "203.0.113.113/32";

pub const USER_FILES: [(&str, &str, Kind); 4] = [
    ("list-general-user.txt", "Мои домены", Kind::Domains),
    ("list-exclude-user.txt", "Мои исключения доменов", Kind::Domains),
    ("ipset-all-user.txt", "Мои IP", Kind::Ips),
    ("ipset-exclude-user.txt", "Мои исключения IP", Kind::Ips),
];

const COMPANIONS: [(&str, &str, &str); 4] = [
    ("--hostlist", "list-general.txt", "list-general-user.txt"),
    ("--hostlist-exclude", "list-exclude.txt", "list-exclude-user.txt"),
    ("--ipset", "ipset-all.txt", "ipset-all-user.txt"),
    ("--ipset-exclude", "ipset-exclude.txt", "ipset-exclude-user.txt"),
];

#[derive(Clone, Debug)]
pub struct ListFile {
    pub path: PathBuf,
    pub label: String,
    pub kind: Kind,
    pub user: bool,
}

pub fn placeholder(kind: Kind) -> &'static str {
    match kind {
        Kind::Domains => DOMAIN_PLACEHOLDER,
        Kind::Ips => IP_PLACEHOLDER,
    }
}

fn is_placeholder(s: &str) -> bool {
    s == DOMAIN_PLACEHOLDER || s == IP_PLACEHOLDER
}

pub fn kind_of(file: &str) -> Option<Kind> {
    let l = file.to_lowercase();
    if !l.ends_with(".txt") {
        return None;
    }
    if l.starts_with("ipset-") {
        Some(Kind::Ips)
    } else if l.starts_with("list-") || l.starts_with("zapret-hosts") {
        Some(Kind::Domains)
    } else {
        None
    }
}

pub fn files(dir: &Path) -> Vec<ListFile> {
    let mut out: Vec<ListFile> = USER_FILES
        .iter()
        .map(|(f, label, kind)| ListFile { path: dir.join(f), label: label.to_string(), kind: *kind, user: true })
        .collect();
    let mut rest = Vec::new();
    if let Ok(rd) = fs::read_dir(dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if USER_FILES.iter().any(|(f, ..)| f.eq_ignore_ascii_case(&name)) {
                continue;
            }
            if let Some(kind) = kind_of(&name) {
                rest.push(ListFile { path: e.path(), label: name.trim_end_matches(".txt").to_string(), kind, user: false });
            }
        }
    }
    rest.sort_by(|a, b| a.label.cmp(&b.label));
    out.extend(rest);
    out
}

#[derive(Clone, Debug, Default)]
pub struct Doc {
    pub lines: Vec<String>,
    pub entries: Vec<usize>,
}

impl Doc {
    pub fn load(path: &Path) -> Doc {
        let text = fs::read_to_string(path).unwrap_or_default();
        let lines: Vec<String> = text.lines().map(|l| l.trim_end_matches('\r').to_string()).collect();
        let mut d = Doc { lines, entries: Vec::new() };
        d.reindex();
        d
    }

    fn reindex(&mut self) {
        self.entries = self
            .lines
            .iter()
            .enumerate()
            .filter(|(_, l)| {
                let t = l.trim();
                !t.is_empty() && !t.starts_with('#') && !is_placeholder(t)
            })
            .map(|(i, _)| i)
            .collect();
    }

    pub fn entry(&self, n: usize) -> &str {
        self.lines[self.entries[n]].trim()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn contains(&self, value: &str) -> bool {
        self.entries.iter().any(|&i| self.lines[i].trim().eq_ignore_ascii_case(value))
    }

    pub fn add(&mut self, values: &[String]) -> usize {
        let mut added = 0;
        let mut fresh: Vec<String> = Vec::new();
        for v in values {
            if self.contains(v) || fresh.iter().any(|f| f.eq_ignore_ascii_case(v)) {
                continue;
            }
            fresh.push(v.clone());
            added += 1;
        }
        if added > 0 {
            self.lines.retain(|l| !is_placeholder(l.trim()));
            while self.lines.last().is_some_and(|l| l.trim().is_empty()) {
                self.lines.pop();
            }
            self.lines.extend(fresh);
            self.reindex();
        }
        added
    }

    pub fn remove(&mut self, n: usize) -> Option<String> {
        let idx = *self.entries.get(n)?;
        let v = self.lines.remove(idx).trim().to_string();
        self.reindex();
        Some(v)
    }

    pub fn save(&self, path: &Path, kind: Kind) -> anyhow::Result<()> {
        let mut lines = self.lines.clone();
        if self.entries.is_empty() && !lines.iter().any(|l| is_placeholder(l.trim())) {
            lines.push(placeholder(kind).to_string());
        }
        let mut text = lines.join("\r\n");
        text.push_str("\r\n");
        let tmp = path.with_extension("txt.tmp");
        fs::write(&tmp, text)?;
        fs::rename(&tmp, path)?;
        Ok(())
    }
}

pub fn normalize_domain(raw: &str) -> Result<String, String> {
    let mut s = raw.trim().to_lowercase();
    if let Some(i) = s.find("://") {
        s = s[i + 3..].to_string();
    }
    if let Some(i) = s.find(['/', '?', '#']) {
        s.truncate(i);
    }
    if let Some(i) = s.rfind('@') {
        s = s[i + 1..].to_string();
    }
    if let Some(i) = s.find(':') {
        s.truncate(i);
    }
    let s = s.trim_start_matches("*.").trim_start_matches('.').trim_end_matches('.').to_string();
    if !s.is_ascii() {
        return Err(format!("{raw}: кириллический домен нужно ввести в punycode (xn--…)"));
    }
    let labels: Vec<&str> = s.split('.').collect();
    let ok = labels.len() >= 2
        && labels.iter().all(|l| {
            !l.is_empty()
                && l.len() <= 63
                && !l.starts_with('-')
                && !l.ends_with('-')
                && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
        && labels.last().is_some_and(|t| t.chars().any(|c| c.is_ascii_alphabetic()));
    if ok { Ok(s) } else { Err(format!("{raw}: не похоже на домен")) }
}

pub fn normalize_ip(raw: &str) -> Result<String, String> {
    let s = raw.trim();
    let (addr, prefix) = match s.split_once('/') {
        Some((a, p)) => (a, Some(p)),
        None => (s, None),
    };
    let ip: IpAddr = addr.parse().map_err(|_| format!("{raw}: не похоже на IP-адрес"))?;
    let max = if ip.is_ipv4() { 32 } else { 128 };
    let prefix = match prefix {
        Some(p) => p.parse::<u8>().ok().filter(|p| *p <= max).ok_or_else(|| format!("{raw}: неверная маска"))?,
        None => max,
    };
    Ok(format!("{ip}/{prefix}"))
}

pub fn parse_input(kind: Kind, text: &str) -> (Vec<String>, Vec<String>) {
    let mut ok = Vec::new();
    let mut bad = Vec::new();
    for token in text.split(|c: char| c.is_whitespace() || c == ',' || c == ';').filter(|t| !t.is_empty()) {
        let r = match kind {
            Kind::Domains => normalize_domain(token),
            Kind::Ips => normalize_ip(token),
        };
        match r {
            Ok(v) if !ok.contains(&v) => ok.push(v),
            Ok(_) => {}
            Err(e) => bad.push(e),
        }
    }
    (ok, bad)
}

pub fn ensure_user_files(dir: &Path) -> anyhow::Result<()> {
    for (f, _, kind) in USER_FILES {
        let p = dir.join(f);
        let empty = fs::read_to_string(&p).map(|t| t.trim().is_empty()).unwrap_or(true);
        if empty {
            fs::write(&p, format!("{}\r\n", placeholder(kind)))?;
        }
    }
    Ok(())
}

pub fn inject_user_lists(args: &str, lists: &str) -> String {
    args.split(" --new ")
        .map(|block| {
            let mut b = block.to_string();
            for (flag, base, user) in COMPANIONS {
                let main = format!("{flag}=\"{lists}{base}\"");
                let extra = format!("{flag}=\"{lists}{user}\"");
                if b.contains(&main) && !b.contains(&extra) {
                    b = b.replacen(&main, &format!("{main} {extra}"), 1);
                }
            }
            b
        })
        .collect::<Vec<_>>()
        .join(" --new ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domains_are_normalized() {
        assert_eq!(normalize_domain("https://Open.Spotify.com:443/path?q=1").unwrap(), "open.spotify.com");
        assert_eq!(normalize_domain("*.discord.gg").unwrap(), "discord.gg");
        assert!(normalize_domain("localhost").is_err());
        assert!(normalize_domain("bad_domain.com").is_err());
        assert!(normalize_domain("сайт.рф").is_err());
    }

    #[test]
    fn ips_are_normalized() {
        assert_eq!(normalize_ip("1.2.3.4").unwrap(), "1.2.3.4/32");
        assert_eq!(normalize_ip("10.0.0.0/8").unwrap(), "10.0.0.0/8");
        assert_eq!(normalize_ip("2606:4700::1111").unwrap(), "2606:4700::1111/128");
        assert!(normalize_ip("1.2.3.4/33").is_err());
        assert!(normalize_ip("1.2.3").is_err());
    }

    #[test]
    fn user_lists_are_injected_once() {
        let l = r"C:\hz\exp-list\";
        let args = format!(
            "--wf-tcp=80 --filter-tcp=443 --hostlist=\"{l}list-general.txt\" --ipset-exclude=\"{l}ipset-exclude.txt\" --new --filter-udp=443 --hostlist=\"{l}list-general.txt\" --hostlist=\"{l}list-general-user.txt\""
        );
        let out = inject_user_lists(&args, l);
        assert_eq!(out.matches("list-general-user.txt").count(), 2);
        assert_eq!(out.matches("ipset-exclude-user.txt").count(), 1);
        assert!(!out.contains("ipset-all-user.txt"));
        assert!(out.contains(&format!("--ipset-exclude=\"{l}ipset-exclude.txt\" --ipset-exclude=\"{l}ipset-exclude-user.txt\"")));
    }

    #[test]
    fn doc_keeps_comments_and_placeholder() {
        let dir = std::env::temp_dir().join(format!("hz-doc-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join("list-general-user.txt");
        fs::write(&p, "# header\r\ndomain.example.abc\r\n").unwrap();
        let mut d = Doc::load(&p);
        assert_eq!(d.len(), 0);
        assert_eq!(d.add(&["a.com".into(), "A.com".into(), "b.com".into()]), 2);
        d.save(&p, Kind::Domains).unwrap();
        assert_eq!(fs::read_to_string(&p).unwrap(), "# header\r\na.com\r\nb.com\r\n");
        let mut d = Doc::load(&p);
        d.remove(0);
        d.remove(0);
        d.save(&p, Kind::Domains).unwrap();
        assert_eq!(fs::read_to_string(&p).unwrap(), "# header\r\ndomain.example.abc\r\n");
        fs::remove_dir_all(&dir).unwrap();
    }
}
