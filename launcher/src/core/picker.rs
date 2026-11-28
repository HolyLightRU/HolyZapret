use super::gamefilter::GameFilter;
use super::paths::Paths;
use super::service::{self, ServiceState};
use super::strategy::{self, Strategy};
use super::{kill_process, run_hidden};
use std::fs;
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectExtendedLimitInformation, SetInformationJobObject,
};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const WORKERS: usize = 10;
const ENGINE_WARMUP: Duration = Duration::from_secs(3);
pub const HOSTING: &str = "Хостинги";
pub const MY_SITES: &str = "Мои сайты";

const HEAVY: &[(&str, &str, &str)] = &[
    ("YouTubeThumb", "YouTube", "https://i.ytimg.com/vi/dQw4w9WgXcQ/maxresdefault.jpg"),
    ("DiscordAsset", "Discord", "https://discord.com/assets/favicon.ico"),
];

const FREEZE_SUITE: &[(&str, &str)] = &[
    ("Cloudflare", "https://cdn.cookielaw.org/scripttemplates/202501.2.0/otBannerSdk.js"),
    ("DigitalOcean", "https://genderize.io/"),
    ("Hetzner", "https://j.dejure.org/jcg/doctrine/doctrine_banner.webp"),
    ("AWS", "https://tms.delta.com/delta/dl_anderson/Bootstrap.js"),
    ("Google Cloud", "https://api.usercentrics.eu/gvl/v3/en.json"),
    ("Fastly", "https://openoffice.apache.org/images/blog/rejected.png"),
    ("Akamai", "https://www.lg.com/lg5-common-gp/library/jquery.min.js"),
    ("OVH", "https://proof.ovh.net/files/1Mb.dat"),
    ("CDN77", "https://cdn.eso.org/images/banner1920/eso2520a.jpg"),
    (
        "Contabo",
        "https://cloudlets.io/wp-content/themes/Avada/includes/lib/assets/fonts/fontawesome/webfonts/fa-solid-900.woff2",
    ),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckKind {
    Tls12,
    Tls13,
    Freeze,
    Quic,
}

#[derive(Clone, Debug)]
pub struct Target {
    pub name: String,
    pub category: String,
    pub url: String,
    pub kind: CheckKind,
}

#[derive(Clone, Debug)]
pub struct Check {
    pub ok: bool,
    pub frozen: bool,
    pub ms: u32,
    pub code: u32,
    pub exit: i32,
    pub skipped: bool,
}

impl Check {
    pub fn failed() -> Check {
        Check { ok: false, frozen: false, ms: 0, code: 0, exit: -1, skipped: false }
    }

    pub fn skipped_from(base: &Check) -> Check {
        Check { skipped: true, ..base.clone() }
    }

    pub fn counted(&self) -> bool {
        !self.skipped || self.ok
    }

    pub fn detail(&self) -> String {
        if self.skipped {
            "обход не трогает этот сайт".into()
        } else if self.frozen {
            "заморозка 16-20 КБ".into()
        } else if self.code > 0 && !self.ok {
            format!("HTTP {}, обрыв при загрузке", self.code)
        } else if self.code > 0 {
            format!("HTTP {}", self.code)
        } else if self.exit >= 0 {
            super::checks::curl_error(self.exit)
        } else {
            "не выполнена".into()
        }
    }
}

pub fn windows_build() -> u32 {
    winreg::RegKey::predef(winreg::enums::HKEY_LOCAL_MACHINE)
        .open_subkey(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion")
        .ok()
        .and_then(|k| k.get_value::<String, _>("CurrentBuildNumber").ok())
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0)
}

pub fn tls13_supported() -> bool {
    windows_build() >= 20348
}

#[derive(Clone, Debug)]
pub struct Outcome {
    pub name: String,
    pub path: Option<PathBuf>,
    pub error: Option<String>,
    pub checks: Vec<Check>,
    pub ray: Option<String>,
}

impl Outcome {
    pub fn score(&self) -> (usize, usize) {
        let run = self.checks.iter().filter(|c| c.counted());
        (run.clone().filter(|c| c.ok).count(), run.count())
    }

    pub fn category(&self, targets: &[Target], cat: &str) -> (usize, usize) {
        let mut ok = 0;
        let mut total = 0;
        for (t, c) in targets.iter().zip(&self.checks) {
            if t.category == cat && c.counted() {
                total += 1;
                ok += c.ok as usize;
            }
        }
        (ok, total)
    }

    pub fn clean_score(&self, noisy: &[bool]) -> usize {
        self.score_without(noisy).0
    }

    pub fn score_without(&self, noisy: &[bool]) -> (usize, usize) {
        let kept = self.checks.iter().enumerate().filter(|(i, c)| c.counted() && !noisy.get(*i).copied().unwrap_or(false));
        kept.fold((0, 0), |(ok, total), (_, c)| (ok + c.ok as usize, total + 1))
    }

    pub fn category_without(&self, targets: &[Target], cat: &str, noisy: &[bool]) -> (usize, usize) {
        let mut ok = 0;
        let mut total = 0;
        for (i, (t, c)) in targets.iter().zip(&self.checks).enumerate() {
            if t.category == cat && c.counted() && !noisy.get(i).copied().unwrap_or(false) {
                total += 1;
                ok += c.ok as usize;
            }
        }
        (ok, total)
    }

    pub fn frozen(&self) -> usize {
        self.checks.iter().filter(|c| c.frozen).count()
    }

    pub fn avg_ms(&self) -> u32 {
        let ok: Vec<u32> = self.checks.iter().filter(|c| c.ok).map(|c| c.ms).collect();
        if ok.is_empty() { 0 } else { ok.iter().sum::<u32>() / ok.len() as u32 }
    }
}

#[derive(Clone, Debug)]
pub enum Event {
    Started { strategies: usize, checks: usize },
    Phase { index: usize, name: String },
    Progress { done: usize },
    Done(Outcome),
    Finished { cancelled: bool, restore: Option<Result<(), String>> },
    Failed(String),
    Lists { added: Vec<String>, open: Vec<String>, error: Option<String> },
}

pub fn noisy_checks(targets: &[Target], results: &[Outcome]) -> Vec<bool> {
    let none = vec![false; targets.len()];
    let Some(base) = results.iter().find(|o| o.path.is_none() && o.checks.len() == targets.len()) else { return none };
    let runs: Vec<&Outcome> =
        results.iter().filter(|o| o.path.is_some() && o.error.is_none() && o.checks.len() == targets.len()).collect();
    if runs.len() < 4 {
        return none;
    }
    targets
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let b = &base.checks[i];
            if t.kind != CheckKind::Freeze || b.ok || b.frozen || runs.iter().any(|o| o.checks[i].frozen) {
                return false;
            }
            let tried: Vec<&&Outcome> = runs.iter().filter(|o| !o.checks[i].skipped).collect();
            let pass = tried.iter().filter(|o| o.checks[i].ok).count();
            tried.len() >= 4 && pass > 0 && pass * 3 < tried.len()
        })
        .collect()
}

pub fn unreachable_checks(targets: &[Target], results: &[Outcome]) -> Vec<bool> {
    let none = vec![false; targets.len()];
    let Some(base) = results.iter().find(|o| o.path.is_none() && o.checks.len() == targets.len()) else { return none };
    let runs: Vec<&Outcome> =
        results.iter().filter(|o| o.path.is_some() && o.error.is_none() && o.checks.len() == targets.len()).collect();
    if runs.is_empty() {
        return none;
    }
    let dead = |c: &Check| !c.ok && !c.frozen;
    (0..targets.len())
        .map(|i| {
            let tried: Vec<&&Outcome> = runs.iter().filter(|o| !o.checks[i].skipped).collect();
            dead(&base.checks[i]) && !tried.is_empty() && tried.iter().all(|o| dead(&o.checks[i]))
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Choice {
    NoBlocks,
    Keep(Option<String>),
    Switch { path: PathBuf, name: String, from: Option<(String, usize)>, score: usize },
}

pub fn choose(targets: &[Target], results: &[Outcome], current: Option<&Path>) -> Choice {
    let mask = either(&noisy_checks(targets, results), &unreachable_checks(targets, results));
    let score = |o: &Outcome| o.clean_score(&mask);
    let ok: Vec<&Outcome> = results.iter().filter(|o| o.error.is_none() && o.checks.len() == targets.len()).collect();
    let Some(best) = ok.iter().copied().max_by_key(|o| (score(o), o.path.is_none(), std::cmp::Reverse(o.frozen()))) else {
        return Choice::Keep(None);
    };
    if score(best) == 0 {
        return Choice::Keep(None);
    }
    let Some(path) = best.path.clone() else { return Choice::NoBlocks };
    let cur = current.and_then(|c| ok.iter().find(|o| o.path.as_deref() == Some(c)));
    match cur {
        Some(c) if c.path == best.path || score(best) <= score(c) => Choice::Keep(Some(c.name.clone())),
        Some(c) => Choice::Switch { path, name: best.name.clone(), from: Some((c.name.clone(), score(c))), score: score(best) },
        None => Choice::Switch { path, name: best.name.clone(), from: None, score: score(best) },
    }
}

pub fn either(a: &[bool], b: &[bool]) -> Vec<bool> {
    a.iter().zip(b).map(|(x, y)| *x || *y).collect()
}

pub fn check_label(t: &Target) -> String {
    match t.kind {
        CheckKind::Freeze => format!("{} / {}", t.category, t.name),
        CheckKind::Tls12 => format!("{} / {} (TLS 1.2)", t.category, t.name),
        CheckKind::Tls13 => format!("{} / {} (TLS 1.3)", t.category, t.name),
        CheckKind::Quic => format!("{} / {} (QUIC)", t.category, t.name),
    }
}

pub fn categories(targets: &[Target]) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    for t in targets {
        if !v.contains(&t.category) {
            v.push(t.category.clone());
        }
    }
    v
}

pub fn parse_targets(text: &str) -> Vec<Target> {
    let mut out = Vec::new();
    let mut category = "Прочее".to_string();
    for line in text.lines() {
        let l = line.trim();
        if let Some(c) = l.strip_prefix("###") {
            let c = c.trim();
            if !c.is_empty() {
                category = c.to_string();
            }
            continue;
        }
        if l.starts_with('#') || l.is_empty() {
            continue;
        }
        let Some((name, value)) = l.split_once('=') else { continue };
        let url = value.trim().trim_matches('"').trim();
        if !(url.starts_with("https://") || url.starts_with("http://")) {
            continue;
        }
        for kind in [CheckKind::Tls12, CheckKind::Tls13] {
            out.push(Target { name: name.trim().to_string(), category: category.clone(), url: url.to_string(), kind });
        }
    }
    out
}

pub fn load_targets(paths: &Paths) -> Vec<Target> {
    let text = fs::read_to_string(paths.root.join("utils").join("targets.txt")).unwrap_or_default();
    let mut t = parse_targets(&text);
    if t.is_empty() {
        t = parse_targets(
            "### Discord\nDiscord = \"https://discord.com\"\n### YouTube\nYouTube = \"https://www.youtube.com\"\nVideo = \"https://redirector.googlevideo.com\"\n",
        );
    }
    for (name, category, url) in HEAVY {
        t.push(Target { name: name.to_string(), category: category.to_string(), url: url.to_string(), kind: CheckKind::Freeze });
    }
    for (host, port) in super::voice::hosts() {
        t.push(Target {
            name: super::voice::label(&host, port),
            category: super::voice::CATEGORY.to_string(),
            url: format!("https://{host}:{port}/"),
            kind: CheckKind::Tls12,
        });
    }
    for url in super::checks::load_my_sites(paths) {
        let name = url.split_once("://").map(|x| x.1).unwrap_or(&url).split('/').next().unwrap_or(&url).to_string();
        for kind in [CheckKind::Tls12, CheckKind::Tls13] {
            t.push(Target { name: name.clone(), category: MY_SITES.into(), url: url.clone(), kind });
        }
    }
    for (name, url) in [("WARP API", "https://api.cloudflareclient.com/"), ("WARP Engage", "https://engage.cloudflareclient.com/")] {
        for kind in [CheckKind::Tls12, CheckKind::Tls13] {
            t.push(Target { name: name.into(), category: "WARP".into(), url: url.into(), kind });
        }
    }
    for (name, url) in FREEZE_SUITE {
        t.push(Target { name: name.to_string(), category: HOSTING.into(), url: url.to_string(), kind: CheckKind::Freeze });
    }
    t
}

pub fn parse_sites(text: &str) -> (Vec<String>, Vec<String>) {
    let (mut ok, mut bad) = (Vec::<String>::new(), Vec::new());
    for tok in text.split(|c: char| c.is_whitespace() || c == ',' || c == ';').filter(|t| !t.is_empty()) {
        match super::checks::normalize_url(tok) {
            Ok(u) if !ok.iter().any(|x| x.eq_ignore_ascii_case(&u)) => ok.push(u),
            Ok(_) => {}
            Err(_) => bad.push(format!("«{tok}» не похоже на адрес сайта")),
        }
    }
    (ok, bad)
}

pub const DNS_FAIL: i32 = 6;

pub fn unresolved(hosts: &[String]) -> Vec<String> {
    use std::net::ToSocketAddrs;
    let (tx, rx) = std::sync::mpsc::channel();
    for h in hosts {
        let (tx, h) = (tx.clone(), h.clone());
        std::thread::spawn(move || {
            let ok = (h.as_str(), 443).to_socket_addrs().is_ok_and(|mut a| a.next().is_some());
            let _ = tx.send((h, ok));
        });
    }
    drop(tx);
    let until = Instant::now() + Duration::from_secs(3);
    let mut missing = Vec::new();
    while let Some(left) = until.checked_duration_since(Instant::now()) {
        match rx.recv_timeout(left) {
            Ok((h, false)) => missing.push(h),
            Ok(_) => {}
            Err(_) => break,
        }
    }
    missing.sort();
    missing.dedup();
    missing
}

pub fn dns_failures(results: &[Outcome], dead: &[bool]) -> Vec<bool> {
    (0..dead.len())
        .map(|i| {
            dead[i]
                && results.iter().filter(|o| o.error.is_none() && o.checks.get(i).is_some_and(|c| !c.skipped)).all(|o| o.checks[i].exit == DNS_FAIL)
                && results.iter().any(|o| o.checks.get(i).is_some_and(|c| c.exit == DNS_FAIL))
        })
        .collect()
}

pub fn host_of(url: &str) -> String {
    url.split_once("://").map(|x| x.1).unwrap_or(url).split('/').next().unwrap_or(url).to_string()
}

pub fn custom_targets(urls: &[String]) -> Vec<Target> {
    let per_site = urls.len() <= 4;
    let mut t = Vec::new();
    for url in urls {
        let name = host_of(url);
        let category = if per_site { name.clone() } else { MY_SITES.to_string() };
        for kind in [CheckKind::Tls12, CheckKind::Tls13] {
            t.push(Target { name: name.clone(), category: category.clone(), url: url.clone(), kind });
        }
    }
    t
}

pub fn site_domains(urls: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for u in urls {
        let d = super::discover::base_domain(&host_of(u));
        if !out.contains(&d) {
            out.push(d);
        }
    }
    out
}

pub fn blocked_sites(targets: &[Target], base: &[Check]) -> (Vec<String>, Vec<String>) {
    let mut blocked: Vec<String> = Vec::new();
    let mut open: Vec<String> = Vec::new();
    for t in targets {
        let failed = targets.iter().zip(base).any(|(u, c)| u.url == t.url && !c.ok);
        let side = if failed { &mut blocked } else { &mut open };
        if !side.contains(&t.url) {
            side.push(t.url.clone());
        }
    }
    (blocked, open)
}

pub fn add_to_user_lists(dirs: &[PathBuf], domains: &[String]) -> anyhow::Result<usize> {
    let mut added = 0;
    for dir in dirs {
        super::lists::ensure_user_files(dir)?;
        let path = dir.join("list-general-user.txt");
        let mut doc = super::lists::Doc::load(&path);
        let n = doc.add(domains);
        if n > 0 {
            doc.save(&path, super::lists::Kind::Domains)?;
            added += n;
        }
    }
    Ok(added)
}

pub fn optimal(_paths: &Paths, strategies: &[Strategy]) -> Vec<usize> {
    strategies
        .iter()
        .enumerate()
        .filter(|(_, s)| !s.id.starts_with("rayoflight:quic-") && !super::ray::EXTRA.iter().any(|e| s.id.strip_prefix("rayoflight:") == Some(*e)))
        .map(|(i, _)| i)
        .collect()
}

pub fn curl_available() -> bool {
    run_hidden("curl.exe", &["--version"]).0
}

fn parse_w(out: &str) -> (u32, u64, f64) {
    let mut it = out.split_whitespace();
    let code = it.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    let size = it.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    let time = it.next().and_then(|v| v.replace(',', ".").parse().ok()).unwrap_or(0.0);
    (code, size, time)
}

pub fn judge(kind: CheckKind, exit: i32, w: &str) -> Check {
    let exit_ok = exit == 0;
    let (code, size, time) = parse_w(w);
    let ms = (time * 1000.0) as u32;
    match kind {
        CheckKind::Tls12 | CheckKind::Tls13 => Check { ok: exit_ok && code > 0, frozen: false, ms, code, exit, skipped: false },
        CheckKind::Quic => Check { ok: exit_ok, frozen: false, ms, code, exit, skipped: false },
        CheckKind::Freeze => {
            let kb = size as f64 / 1024.0;
            let frozen = !exit_ok && (14.0..=22.0).contains(&kb);
            Check { ok: code > 0 && (exit_ok || kb > 22.0), frozen, ms, code, exit, skipped: false }
        }
    }
}

pub(crate) fn run_check(t: &Target) -> Check {
    let c = curl_check(t);
    if t.category == super::voice::CATEGORY && c.exit == 6 {
        return Check::skipped_from(&Check::failed());
    }
    if t.kind == CheckKind::Freeze && !c.ok && !c.frozen { curl_check(t) } else { c }
}

pub const QUIC: &str = "QUIC";
const QUIC_SITES: [&str; 4] = ["www.youtube.com", "redirector.googlevideo.com", "www.google.com", "discord.com"];
const QUIC_FAKE: &[u8] = include_bytes!("../../../bin/quic_initial_www_google_com.bin");

pub fn quic_targets() -> Vec<Target> {
    QUIC_SITES.iter().map(|h| Target { name: h.to_string(), category: QUIC.into(), url: format!("https://{h}/"), kind: CheckKind::Quic }).collect()
}

pub fn quic_probe(host: &str) -> Option<Vec<u8>> {
    use rayoflight::quic;
    let (i, hello) = quic::hello_of(QUIC_FAKE)?;
    let named = quic::with_sni(&hello, host)?;
    let mut dcid = [0u8; 8];
    rayoflight::sys::random(&mut dcid);
    let mut frames = Vec::new();
    quic::crypto_frame(&mut frames, 0, &named);
    Some(quic::seal_initial(&dcid, &i.scid, &[], 0, &frames, 1250))
}

fn quic_check(t: &Target) -> Check {
    use std::net::{ToSocketAddrs, UdpSocket};
    let host = host_of(&t.url);
    let start = Instant::now();
    let done = |ok: bool, exit: i32| Check { ok, frozen: false, ms: start.elapsed().as_millis() as u32, code: 0, exit, skipped: false };
    let Some(addr) = (host.as_str(), 443).to_socket_addrs().ok().and_then(|mut a| a.find(|x| x.is_ipv4())) else { return done(false, 6) };
    let Some(pkt) = quic_probe(&host) else { return done(false, -1) };
    let Ok(sock) = UdpSocket::bind("0.0.0.0:0") else { return done(false, 7) };
    if sock.connect(addr).is_err() {
        return done(false, 7);
    }
    let mut buf = [0u8; 2048];
    for wait in [1500u64, 2500] {
        if sock.send(&pkt).is_err() {
            return done(false, 7);
        }
        let until = Instant::now() + Duration::from_millis(wait);
        while let Some(left) = until.checked_duration_since(Instant::now()).filter(|d| !d.is_zero()) {
            let _ = sock.set_read_timeout(Some(left));
            match sock.recv(&mut buf) {
                Ok(n) if rayoflight::quic::is_long_header(&buf[..n]) => return done(true, 0),
                Ok(_) => continue,
                Err(_) => break,
            }
        }
    }
    done(false, 28)
}

pub fn record_probe(targets: &[Target], cancel: &AtomicBool) -> Option<String> {
    use super::checks::{Shape, tls13_probe_as};
    let mut hosts: Vec<String> = targets.iter().filter(|t| matches!(t.kind, CheckKind::Tls12 | CheckKind::Tls13)).map(|t| host_of(&t.url)).collect();
    hosts.sort();
    hosts.dedup();
    if hosts.is_empty() || cancel.load(Ordering::Relaxed) {
        return None;
    }
    let shapes = [Shape::Plain, Shape::Records, Shape::RecordsApart];
    let results: Vec<[u8; 3]> = std::thread::scope(|sc| {
        let jobs: Vec<_> = hosts
            .iter()
            .map(|h| {
                sc.spawn(move || {
                    let mut won = [0u8; 3];
                    for _ in 0..PROBE_TRIES {
                        if cancel.load(Ordering::Relaxed) {
                            break;
                        }
                        for (i, s) in shapes.iter().enumerate() {
                            if matches!(tls13_probe_as(h, Duration::from_secs(3), *s).0, Ok(_)) {
                                won[i] += 1;
                            }
                        }
                    }
                    won
                })
            })
            .collect();
        jobs.into_iter().map(|j| j.join().unwrap_or([0; 3])).collect()
    });
    if cancel.load(Ordering::Relaxed) {
        return None;
    }
    Some(record_summary(&hosts, &results))
}

const PROBE_TRIES: u8 = 3;

pub fn record_summary(hosts: &[String], results: &[[u8; 3]]) -> String {
    let steady = |n: u8| n * 2 > PROBE_TRIES;
    let count = |i: usize| results.iter().filter(|r| steady(r[i])).count();
    let only: Vec<&str> = hosts.iter().zip(results).filter(|(_, r)| r[0] == 0 && (steady(r[1]) || steady(r[2]))).map(|(h, _)| h.as_str()).collect();
    let shaky = results.iter().filter(|r| r.iter().any(|&n| n > 0 && n < PROBE_TRIES)).count();
    let mut s = format!(
        "Проба разреза на TLS-записи без обхода (TLS 1.3, сайтов {}, по {} попытки каждого вида; считается сайт, ответивший хотя бы в 2 из 3): обычный ClientHello — ответили {}; две записи в одном пакете — {}; две записи в двух пакетах — {}; ответы нестабильны у {}",
        hosts.len(),
        PROBE_TRIES,
        count(0),
        count(1),
        count(2),
        shaky
    );
    if !only.is_empty() {
        s.push_str(&format!(". Обычный ни разу, а разрез стабильно: {}", only.join(", ")));
    }
    s
}

fn hello_check(t: &Target) -> Check {
    let (r, ms) = super::checks::tls13_probe(&host_of(&t.url), Duration::from_secs(5));
    let (ok, exit) = match r {
        Ok(_) => (true, 0),
        Err(e) => (false, e),
    };
    Check { ok, frozen: false, ms: ms as u32, code: 0, exit, skipped: false }
}

fn curl_check(t: &Target) -> Check {
    if t.kind == CheckKind::Quic {
        return quic_check(t);
    }
    if t.kind == CheckKind::Tls13 && !tls13_supported() {
        return hello_check(t);
    }
    let mut args: Vec<&str> = vec!["-s", "-o", "NUL", "-A", "Mozilla/5.0 (Windows NT 10.0; Win64; x64)"];
    match t.kind {
        CheckKind::Tls12 => args.extend(["-I", "-m", "5", "--tlsv1.2", "--tls-max", "1.2", "-w", "%{http_code} 0 %{time_total}"]),
        CheckKind::Tls13 => args.extend(["-I", "-m", "5", "--tlsv1.3", "-w", "%{http_code} 0 %{time_total}"]),
        CheckKind::Freeze => args.extend([
            "-L",
            "-m",
            "6",
            "--range",
            "0-262143",
            "-w",
            "%{http_code} %{size_download} %{time_total}",
        ]),
        CheckKind::Quic => {}
    }
    args.push(&t.url);
    let out = Command::new("curl.exe").args(&args).creation_flags(CREATE_NO_WINDOW).output();
    match out {
        Ok(o) => judge(t.kind, o.status.code().unwrap_or(-1), &String::from_utf8_lossy(&o.stdout)),
        Err(_) => Check::failed(),
    }
}

fn coverage_skips(targets: &[Target], ips: &[Vec<std::net::IpAddr>], args: &str, cache: &mut super::coverage::Cache) -> Vec<bool> {
    targets
        .iter()
        .zip(ips)
        .map(|(t, ips)| t.category == HOSTING && !super::coverage::touches(args, &host_of(&t.url), ips, cache))
        .collect()
}

fn resolve_hosting(targets: &[Target]) -> Vec<Vec<std::net::IpAddr>> {
    use std::net::ToSocketAddrs;
    std::thread::scope(|s| {
        let jobs: Vec<_> = targets
            .iter()
            .map(|t| {
                s.spawn(move || {
                    if t.category != HOSTING {
                        return Vec::new();
                    }
                    (host_of(&t.url).as_str(), 443).to_socket_addrs().map(|a| a.map(|x| x.ip()).collect()).unwrap_or_default()
                })
            })
            .collect();
        jobs.into_iter().map(|j| j.join().unwrap_or_default()).collect()
    })
}

fn run_suite(targets: &Arc<Vec<Target>>, cancel: &Arc<AtomicBool>, emit: &(impl Fn(Event) + Sync)) -> Vec<Check> {
    run_suite_skipping(targets, &vec![false; targets.len()], &[], cancel, emit, WORKERS)
}

fn run_suite_skipping(
    targets: &Arc<Vec<Target>>,
    skip: &[bool],
    base: &[Check],
    cancel: &Arc<AtomicBool>,
    emit: &(impl Fn(Event) + Sync),
    workers: usize,
) -> Vec<Check> {
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let results: Mutex<Vec<Option<Check>>> = Mutex::new(vec![None; targets.len()]);
    std::thread::scope(|s| {
        for _ in 0..workers.min(targets.len()) {
            s.spawn(|| {
                loop {
                    if cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(t) = targets.get(i) else { break };
                    let c = match (skip.get(i).copied().unwrap_or(false), base.get(i)) {
                        (true, Some(b)) => Check::skipped_from(b),
                        _ => run_check(t),
                    };
                    results.lock().unwrap()[i] = Some(c);
                    let d = done.fetch_add(1, Ordering::Relaxed) + 1;
                    emit(Event::Progress { done: d });
                }
            });
        }
    });
    results
        .into_inner()
        .unwrap()
        .into_iter()
        .map(|c| c.unwrap_or_else(Check::failed))
        .collect()
}

fn job() -> isize {
    *JOB.get_or_init(|| unsafe {
        let h = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if h.is_null() {
            return 0;
        }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        SetInformationJobObject(
            h,
            JobObjectExtendedLimitInformation,
            &info as *const _ as _,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        h as isize
    })
}

static JOB: OnceLock<isize> = OnceLock::new();

pub fn log_path(name: &str) -> PathBuf {
    let safe: String = name.chars().map(|c| if c.is_alphanumeric() || c == '-' { c } else { '_' }).collect();
    std::env::temp_dir().join("holyzapret-picker").join(format!("{safe}.log"))
}

static COMPANION: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);
static RAY_SUMMARY: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

fn start_engine(paths: &Paths, s: &Strategy, game_filter: &GameFilter, _rules: bool) -> anyhow::Result<Child> {
    super::lists::ensure_user_files(&paths.exp_lists())?;
    let exp = s.ray.as_ref().map(|r| r.0.clone()).unwrap_or_else(|| "adaptive".into());
    let ray = super::ray::exe(paths).ok_or_else(|| anyhow::anyhow!("нет RayOfLight.exe"))?;
    let live = log_path(&format!("{}-ray", s.name)).with_extension("json");
    let _ = fs::remove_file(&live);
    let child = start_raw(&ray, &paths.bin(), &super::ray::cli_args(paths, &exp, &live, game_filter), &s.name)?;
    if let Ok(mut g) = COMPANION.lock() {
        *g = Some(live);
    }
    Ok(child)
}

fn stop_companion() {
    let Some(live) = COMPANION.lock().ok().and_then(|mut g| g.take()) else { return };
    std::thread::sleep(Duration::from_millis(super::ray::PICK_LIVE_MS + 150));
    let summary = fs::read_to_string(&live).ok().and_then(|t| super::ray::parse_live(&t)).map(|l| super::ray::pick_summary(&l));
    if let Ok(mut g) = RAY_SUMMARY.lock() {
        *g = Some(summary.unwrap_or_else(|| "RayOfLight: сводки нет, движок не успел её записать".into()));
    }
}

fn take_ray_summary() -> Option<String> {
    RAY_SUMMARY.lock().ok().and_then(|mut g| g.take())
}

pub(crate) fn start_raw(engine: &std::path::Path, bin: &std::path::Path, args: &str, name: &str) -> anyhow::Result<Child> {
    let path = log_path(name);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let log = fs::File::create(path)?;
    let child = Command::new(engine)
        .raw_arg(args)
        .current_dir(bin)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log))
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()?;
    let j = job();
    if j != 0 {
        unsafe {
            AssignProcessToJobObject(j as _, child.as_raw_handle() as _);
        }
    }
    Ok(child)
}

pub fn engine_error(log: &str) -> String {
    let noise = ["github version", "we have ", "loading", "loaded", "windivert initialized", "running as"];
    let lines: Vec<&str> = log
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .filter(|l| !noise.iter().any(|n| l.to_lowercase().starts_with(n)))
        .collect();
    let tail = lines[lines.len().saturating_sub(3)..].join(" · ");
    if tail.is_empty() { "движок завершился сразу после запуска".into() } else { tail }
}

pub(crate) fn warm_up(child: &mut Child, name: &str, cancel: &AtomicBool) -> Result<(), String> {
    let t = Instant::now();
    while t.elapsed() < ENGINE_WARMUP {
        if cancel.load(Ordering::Relaxed) {
            return Ok(());
        }
        if let Ok(Some(_)) = child.try_wait() {
            return Err(engine_error(&fs::read_to_string(log_path(name)).unwrap_or_default()));
        }
        if companion_exited() {
            let log = fs::read_to_string(log_path(&format!("{name}-ray"))).unwrap_or_default();
            let last = log.lines().map(str::trim).rfind(|l| !l.is_empty()).unwrap_or("движок завершился сразу после запуска");
            return Err(format!("RayOfLight: {last}"));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}

fn companion_exited() -> bool {
    false
}

pub(crate) fn stop_engine(child: Option<Child>) {
    stop_companion();
    if let Some(mut c) = child {
        let _ = c.kill();
        let _ = c.wait();
    }
    kill_process("HolyZapret.exe");
    kill_process("winws.exe");
    std::thread::sleep(Duration::from_millis(400));
}

#[allow(clippy::too_many_arguments)]
pub fn run(
    paths: Paths,
    strategies: Vec<Strategy>,
    targets: Vec<Target>,
    game_filter: GameFilter,
    add_to: Vec<PathBuf>,
    cancel: Arc<AtomicBool>,
    emit: impl Fn(Event) + Sync,
) {
    if !curl_available() {
        emit(Event::Failed("curl.exe не найден: он входит в Windows 10 1803 и новее".into()));
        return;
    }
    let was_running = service::query(service::SERVICE) == ServiceState::Running;
    let had_divert = service::query("WinDivert") != ServiceState::NotInstalled;
    if was_running {
        run_hidden("net", &["stop", service::SERVICE]);
    }
    stop_engine(None);
    let targets = Arc::new(targets);
    emit(Event::Started { strategies: strategies.len(), checks: targets.len() });

    emit(Event::Phase { index: 0, name: "Без обхода".into() });
    let base = run_suite(&targets, &cancel, &emit);
    let hosting_ips = resolve_hosting(&targets);
    let mut cover = super::coverage::Cache::default();
    if !cancel.load(Ordering::Relaxed) {
        let probe = if super::TEST_BUILD { record_probe(&targets, &cancel) } else { None };
        emit(Event::Done(Outcome { name: "Без обхода".into(), path: None, error: None, checks: base.clone(), ray: probe }));
        if !add_to.is_empty() {
            let (blocked, open) = blocked_sites(&targets, &base);
            let added = site_domains(&blocked);
            let error = if added.is_empty() { None } else { add_to_user_lists(&add_to, &added).err().map(|e| e.to_string()) };
            emit(Event::Lists { added, open: site_domains(&open), error });
        }
    }

    for (i, s) in strategies.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        emit(Event::Phase { index: i + 1, name: s.name.clone() });
        let _ = take_ray_summary();
        let mut outcome = Outcome { name: s.name.clone(), path: Some(s.path.clone()), error: None, checks: Vec::new(), ray: None };
        let skip = strategy::extract_args(s, &paths, &game_filter)
            .map(|a| coverage_skips(&targets, &hosting_ips, &a, &mut cover))
            .unwrap_or_else(|_| vec![false; targets.len()]);
        match start_engine(&paths, s, &game_filter, false) {
            Ok(mut child) => match warm_up(&mut child, &s.name, &cancel) {
                Ok(()) => {
                    outcome.checks = run_suite_skipping(&targets, &skip, &base, &cancel, &emit, WORKERS);
                    recheck(&targets, &base, &mut outcome.checks, &cancel);
                    if blackout(&base, &outcome.checks) && !cancel.load(Ordering::Relaxed) {
                        stop_engine(Some(child));
                        child = match start_engine(&paths, s, &game_filter, false) {
                            Ok(c) => c,
                            Err(e) => {
                                outcome.error = Some(e.to_string());
                                emit(Event::Done(Outcome { checks: vec![Check::failed(); targets.len()], ..outcome }));
                                continue;
                            }
                        };
                        if warm_up(&mut child, &s.name, &cancel).is_ok() {
                            outcome.checks = run_suite_skipping(&targets, &skip, &base, &cancel, &emit, WORKERS);
                            recheck(&targets, &base, &mut outcome.checks, &cancel);
                        }
                    }
                    stop_engine(Some(child));
                }
                Err(e) => {
                    outcome.error = Some(e);
                    stop_engine(Some(child));
                }
            },
            Err(e) => outcome.error = Some(e.to_string()),
        }
        if outcome.error.is_some() {
            outcome.checks = vec![Check::failed(); targets.len()];
        }
        outcome.ray = take_ray_summary().filter(|_| s.ray.is_some());
        if !cancel.load(Ordering::Relaxed) || outcome.error.is_some() {
            emit(Event::Done(outcome));
        }
    }
    stop_engine(None);
    let restore = was_running.then(|| service::start().map_err(|e| e.to_string()));
    if !was_running && !had_divert {
        let _ = service::unload_windivert();
    }
    emit(Event::Finished { cancelled: cancel.load(Ordering::Relaxed), restore });
}

fn regressions(base: &[Check], checks: &[Check]) -> Vec<usize> {
    (0..checks.len().min(base.len())).filter(|&i| base[i].ok && !checks[i].ok && !checks[i].skipped).collect()
}

fn blackout(base: &[Check], checks: &[Check]) -> bool {
    let tried: Vec<&Check> = checks.iter().filter(|c| !c.skipped).collect();
    base.iter().any(|c| c.ok) && !tried.is_empty() && tried.iter().all(|c| !c.ok)
}

fn recheck(targets: &[Target], base: &[Check], checks: &mut [Check], cancel: &AtomicBool) {
    let again = regressions(base, checks);
    if again.is_empty() || cancel.load(Ordering::Relaxed) {
        return;
    }
    std::thread::sleep(Duration::from_secs(1));
    let fresh: Vec<(usize, Check)> = std::thread::scope(|s| {
        let jobs: Vec<_> = again.iter().map(|&i| s.spawn(move || (i, run_check(&targets[i])))).collect();
        jobs.into_iter().filter_map(|j| j.join().ok()).collect()
    });
    for (i, c) in fresh {
        if c.ok {
            checks[i] = c;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_blocked_sites_go_to_lists() {
        let t = |url: &str, kind| Target { name: url.into(), category: "x".into(), url: url.into(), kind };
        let targets = vec![
            t("https://soundcloud.com", CheckKind::Tls12),
            t("https://soundcloud.com", CheckKind::Tls13),
            t("https://store.steampowered.com", CheckKind::Tls12),
            t("https://store.steampowered.com", CheckKind::Tls13),
        ];
        let c = |ok: bool| Check { ok, frozen: false, ms: 10, code: if ok { 200 } else { 0 }, exit: if ok { 0 } else { 28 }, skipped: false };
        let (blocked, open) = blocked_sites(&targets, &[c(true), c(false), c(true), c(true)]);
        assert_eq!(blocked, vec!["https://soundcloud.com".to_string()]);
        assert_eq!(open, vec!["https://store.steampowered.com".to_string()]);
        assert_eq!(site_domains(&open), vec!["steampowered.com".to_string()]);
    }

    #[test]
    fn skipped_checks_do_not_count() {
        let t = |name: &str| Target { name: name.into(), category: HOSTING.into(), url: format!("https://{name}/"), kind: CheckKind::Freeze };
        let targets = vec![t("a.com"), t("b.com")];
        let ok = Check { ok: true, frozen: false, ms: 10, code: 200, exit: 0, skipped: false };
        let dead = Check::failed();
        let out = |path: Option<&str>, checks: Vec<Check>| Outcome { name: "s".into(), path: path.map(PathBuf::from), error: None, checks, ray: None };
        let general = out(Some("g"), vec![Check::skipped_from(&ok), Check::skipped_from(&dead)]);
        assert_eq!(general.score(), (1, 1));
        assert_eq!(general.category(&targets, HOSTING), (1, 1));
        assert_eq!(general.score_without(&[false, false]), (1, 1));
        let broad = out(Some("b"), vec![ok.clone(), ok.clone()]);
        assert_eq!(broad.score(), (2, 2));
        let base = out(None, vec![ok.clone(), dead.clone()]);
        let results = vec![base, general.clone(), out(Some("h"), vec![Check::skipped_from(&ok), Check::skipped_from(&dead)])];
        assert_eq!(unreachable_checks(&targets, &results), vec![false, false]);
        assert!(regressions(&[ok.clone(), ok.clone()], &general.checks).is_empty());
        assert!(!blackout(&[ok.clone(), ok.clone()], &[Check::skipped_from(&ok), Check::skipped_from(&dead)]));
        assert_eq!(Check::skipped_from(&dead).detail(), "обход не трогает этот сайт");
    }

    #[test]
    #[ignore]
    fn real_strategies_hosting_coverage() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf();
        let paths = Paths { root };
        let targets: Vec<Target> = load_targets(&paths).into_iter().filter(|t| t.category == HOSTING).collect();
        let ips = resolve_hosting(&targets);
        let mut cache = super::super::coverage::Cache::default();
        for s in strategy::discover(&paths) {
            let Ok(args) = strategy::extract_args(&s, &paths, &GameFilter::default()) else { continue };
            let skip = coverage_skips(&targets, &ips, &args, &mut cache);
            let touched: Vec<&str> = targets.iter().zip(&skip).filter(|(_, s)| !**s).map(|(t, _)| t.name.as_str()).collect();
            println!("{:<45} {}", s.name, touched.join(", "));
        }
    }

    #[test]
    fn regressions_and_blackouts() {
        let c = |ok: bool| Check { ok, frozen: false, ms: 10, code: if ok { 200 } else { 0 }, exit: if ok { 0 } else { 28 }, skipped: false };
        let base = vec![c(true), c(false), c(true)];
        assert_eq!(regressions(&base, &[c(false), c(false), c(true)]), vec![0]);
        assert!(blackout(&base, &[c(false), c(false), c(false)]));
        assert!(!blackout(&base, &[c(false), c(true), c(false)]));
        assert!(!blackout(&[c(false), c(false)], &[c(false), c(false)]));
    }

    #[test]
    fn lucky_timeouts_are_noise() {
        let t = |name: &str, kind| Target { name: name.into(), category: "x".into(), url: String::new(), kind };
        let targets = vec![t("yt", CheckKind::Tls12), t("cdn77", CheckKind::Freeze), t("hetzner", CheckKind::Freeze)];
        let c = |ok: bool, frozen: bool| Check { ok, frozen, ms: 10, code: if ok { 200 } else { 0 }, exit: if ok { 0 } else { 28 }, skipped: false };
        let run = |name: &str, yt, cdn, het| Outcome {
            name: name.into(),
            path: Some(PathBuf::from(name)),
            error: None,
            checks: vec![c(yt, false), c(cdn, false), c(het, false)],
            ray: None,
        };
        let base = Outcome { name: "base".into(), path: None, error: None, checks: vec![c(false, false), c(false, false), c(false, true)], ray: None };
        let mut results = vec![base, run("lucky", true, true, true), run("b", true, false, true), run("c", true, false, false), run("d", false, false, false)];
        assert_eq!(noisy_checks(&targets, &results), vec![false, true, false]);
        let noisy = noisy_checks(&targets, &results);
        assert_eq!(results[1].clean_score(&noisy), 2);
        assert_eq!(results[1].score_without(&noisy), (2, 2));
        assert_eq!(results[0].score_without(&noisy), (0, 2));
        assert_eq!(results[1].category_without(&targets, "x", &noisy), (2, 2));
        results.truncate(3);
        assert_eq!(noisy_checks(&targets, &results), vec![false, false, false]);
    }

    #[test]
    fn blocked_everywhere_is_marked() {
        let t = |name: &str| Target { name: name.into(), category: "x".into(), url: String::new(), kind: CheckKind::Freeze };
        let targets = vec![t("yt"), t("cdn77"), t("frozen"), t("ok")];
        let c = |ok: bool, frozen: bool| Check { ok, frozen, ms: 10, code: if ok { 200 } else { 0 }, exit: if ok { 0 } else { 28 }, skipped: false };
        let out = |path: Option<&str>, checks: Vec<Check>| Outcome { name: "s".into(), path: path.map(PathBuf::from), error: None, checks, ray: None };
        let base = out(None, vec![c(false, false), c(false, false), c(false, true), c(true, false)]);
        let a = out(Some("a"), vec![c(true, false), c(false, false), c(false, true), c(true, false)]);
        let b = out(Some("b"), vec![c(false, false), c(false, false), c(false, true), c(true, false)]);
        let mut results = vec![base];
        assert_eq!(unreachable_checks(&targets, &results), vec![false; 4]);
        results.extend([a, b]);
        let u = unreachable_checks(&targets, &results);
        assert_eq!(u, vec![false, true, false, false]);
        assert_eq!(either(&u, &[true, false, false, false]), vec![true, true, false, false]);
        assert_eq!(results[1].score_without(&u), (2, 3));
        assert_eq!(check_label(&targets[1]), "x / cdn77");
    }

    #[test]
    fn custom_sites_become_targets() {
        let (ok, bad) = parse_sites("rutracker.org, https://www.kinopoisk.ru/film/1\nhttps://rutracker.org/\nftp://x");
        assert_eq!(ok, vec!["https://rutracker.org/".to_string(), "https://www.kinopoisk.ru/film/1".to_string()]);
        assert_eq!(bad.len(), 1);
        let t = custom_targets(&ok);
        assert!(t.iter().all(|x| x.kind != CheckKind::Freeze));
        assert_eq!(categories(&t), vec!["rutracker.org".to_string(), "www.kinopoisk.ru".to_string()]);
        assert_eq!(site_domains(&ok), vec!["rutracker.org".to_string(), "kinopoisk.ru".to_string()]);
    }

    #[test]
    fn targets_parsed_with_categories() {
        let t = parse_targets(
            "# c\n### Discord\nDiscordMain = \"https://discord.com\"\n### DNS\nX = \"PING:1.1.1.1\"\n### YouTube\nYT = \"https://www.youtube.com\"\n",
        );
        assert_eq!(t.len(), 4);
        assert_eq!(t[0].category, "Discord");
        assert_eq!(t[0].kind, CheckKind::Tls12);
        assert_eq!(t[1].kind, CheckKind::Tls13);
        assert_eq!(t[3].category, "YouTube");
        assert_eq!(categories(&t), vec!["Discord".to_string(), "YouTube".to_string()]);
    }

    #[test]
    fn judging() {
        assert!(judge(CheckKind::Tls13, 0, "200 0 0.150").ok);
        assert_eq!(judge(CheckKind::Tls13, 0, "200 0 0,150").ms, 150);
        assert!(!judge(CheckKind::Tls12, 28, "000 0 5.0").ok);
        let f = judge(CheckKind::Freeze, 28, "206 17408 6.0");
        assert!(!f.ok && f.frozen);
        let e = judge(CheckKind::Freeze, 28, "000 0 6.0");
        assert!(!e.ok && !e.frozen);
        assert!(judge(CheckKind::Freeze, 0, "206 262144 1.2").ok);
        assert!(judge(CheckKind::Freeze, 28, "206 81506 8.0").ok);
    }

    #[test]
    #[ignore]
    fn quic_check_live() {
        let r: Vec<(String, bool, u32)> = quic_targets().iter().map(|t| { let c = run_check(t); (t.name.clone(), c.ok, c.ms) }).collect();
        assert!(r.iter().all(|x| x.1), "{r:?}");
    }

    #[test]
    fn record_probe_summary_names_sites_that_only_split_opens() {
        let hosts = vec!["a.com".to_string(), "b.com".to_string(), "c.com".to_string()];
        let s = record_summary(&hosts, &[[3, 3, 3], [0, 2, 1], [1, 0, 0]]);
        assert!(s.contains("обычный ClientHello — ответили 1; две записи в одном пакете — 2; две записи в двух пакетах — 1; ответы нестабильны у 2"), "{s}");
        assert!(s.ends_with("Обычный ни разу, а разрез стабильно: b.com"), "{s}");
        assert!(!record_summary(&hosts[..1], &[[3, 3, 3]]).contains("разрез стабильно"));
        assert!(!record_summary(&hosts[..1], &[[0, 1, 1]]).contains("разрез стабильно"));
    }

    #[test]
    fn missing_addresses_are_found() {
        let m = unresolved(&["localhost".to_string(), "no-such-host.invalid".to_string()]);
        assert_eq!(m, vec!["no-such-host.invalid".to_string()]);
        let c = |exit: i32| Check { ok: false, frozen: false, ms: 0, code: 0, exit, skipped: false };
        let o = |a: i32, b: i32| Outcome { name: "x".into(), path: None, error: None, checks: vec![c(a), c(b)], ray: None };
        assert_eq!(dns_failures(&[o(6, 28), o(6, 28)], &[true, true]), vec![true, false]);
        assert_eq!(dns_failures(&[o(6, 6), o(28, 6)], &[true, false]), vec![false, false]);
    }

    #[test]
    fn quic_probe_names_the_site() {
        let p = quic_probe("www.youtube.com").unwrap();
        assert_eq!(p.len(), 1250);
        let (_, hello) = rayoflight::quic::hello_of(&p).unwrap();
        assert_eq!(rayoflight::quic::sni(&hello).as_deref(), Some("www.youtube.com"));
        assert!(quic_targets().iter().all(|t| t.kind == CheckKind::Quic && t.category == QUIC));
        assert!(judge(CheckKind::Quic, 0, "").ok);
    }

    #[test]
    fn engine_errors_skip_noise() {
        let log = "github version v72.2 (abc)\r\n\r\nwe have 9 user defined desync profile(s)\r\nLoading hostlist C:/x.txt\r\nloading plain text list\r\ncould not read C:/bin/x.bin\r\n";
        assert_eq!(engine_error(log), "could not read C:/bin/x.bin");
        assert_eq!(engine_error("github version v72.2\n"), "движок завершился сразу после запуска");
    }

    #[test]
    fn outcome_scores() {
        let targets = parse_targets("### A\nA = \"https://a.com\"\n### B\nB = \"https://b.com\"\n");
        let c = |ok, ms| Check { ok, frozen: false, ms, code: 0, exit: 0, skipped: false };
        let o = Outcome { name: "x".into(), path: None, error: None, checks: vec![c(true, 100), c(true, 300), c(false, 0), c(true, 200)], ray: None };
        assert_eq!(o.score(), (3, 4));
        assert_eq!(o.category(&targets, "B"), (1, 2));
        assert_eq!(o.avg_ms(), 200);
    }
}


