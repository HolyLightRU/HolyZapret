use super::gamefilter::GameFilter;
use super::paths::Paths;
use super::service::{self, ServiceState};
use super::strategy::{self, Strategy};
use super::{kill_process, run_hidden};
use std::fs;
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
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

const FREEZE_SUITE: &[(&str, &str)] = &[
    ("Cloudflare", "https://cdn.cookielaw.org/scripttemplates/202501.2.0/otBannerSdk.js"),
    ("DigitalOcean", "https://genderize.io/"),
    ("Hetzner", "https://j.dejure.org/jcg/doctrine/doctrine_banner.webp"),
    ("AWS", "https://tms.delta.com/delta/dl_anderson/Bootstrap.js"),
    ("Google Cloud", "https://api.usercentrics.eu/gvl/v3/en.json"),
    ("Fastly", "https://openoffice.apache.org/images/blog/rejected.png"),
    ("Akamai", "https://www.lg.com/lg5-common-gp/library/jquery.min.js"),
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
}

#[derive(Clone, Debug)]
pub struct Outcome {
    pub name: String,
    pub path: Option<PathBuf>,
    pub error: Option<String>,
    pub checks: Vec<Check>,
}

impl Outcome {
    pub fn score(&self) -> (usize, usize) {
        (self.checks.iter().filter(|c| c.ok).count(), self.checks.len())
    }

    pub fn category(&self, targets: &[Target], cat: &str) -> (usize, usize) {
        let mut ok = 0;
        let mut total = 0;
        for (t, c) in targets.iter().zip(&self.checks) {
            if t.category == cat {
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

pub fn optimal(paths: &Paths, strategies: &[Strategy]) -> Vec<usize> {
    let text = fs::read_to_string(paths.root.join("utils").join("optimal_strategies.txt")).unwrap_or_default();
    let mut out = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let want = paths.root.join(line.replace('/', "\\"));
        if let Some(i) = strategies.iter().position(|s| s.path == want) {
            if !out.contains(&i) {
                out.push(i);
            }
        }
    }
    out
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

pub fn judge(kind: CheckKind, exit_ok: bool, w: &str) -> Check {
    let (code, size, time) = parse_w(w);
    let ms = (time * 1000.0) as u32;
    match kind {
        CheckKind::Tls12 | CheckKind::Tls13 => Check { ok: exit_ok && code > 0, frozen: false, ms },
        CheckKind::Freeze => {
            let kb = size as f64 / 1024.0;
            let frozen = !exit_ok && (14.0..=22.0).contains(&kb);
            Check { ok: code > 0 && (exit_ok || kb > 22.0), frozen, ms }
        }
    }
}

fn run_check(t: &Target) -> Check {
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
    }
    args.push(&t.url);
    let out = Command::new("curl.exe").args(&args).creation_flags(CREATE_NO_WINDOW).output();
    match out {
        Ok(o) => judge(t.kind, o.status.success(), &String::from_utf8_lossy(&o.stdout)),
        Err(_) => Check { ok: false, frozen: false, ms: 0 },
    }
}

fn run_suite(targets: &Arc<Vec<Target>>, cancel: &Arc<AtomicBool>, emit: &(impl Fn(Event) + Sync)) -> Vec<Check> {
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let results: Mutex<Vec<Option<Check>>> = Mutex::new(vec![None; targets.len()]);
    std::thread::scope(|s| {
        for _ in 0..WORKERS.min(targets.len()) {
            s.spawn(|| {
                loop {
                    if cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(t) = targets.get(i) else { break };
                    let c = run_check(t);
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
        .map(|c| c.unwrap_or(Check { ok: false, frozen: false, ms: 0 }))
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

fn start_engine(paths: &Paths, s: &Strategy, game_filter: &GameFilter) -> anyhow::Result<Child> {
    super::lists::ensure_user_files(&strategy::lists_dir(s, paths))?;
    let args = strategy::extract_args(s, paths, game_filter)?;
    let path = log_path(&s.name);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let log = fs::File::create(path)?;
    let child = Command::new(&s.engine)
        .raw_arg(args)
        .current_dir(&s.bin)
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

fn warm_up(child: &mut Child, name: &str, cancel: &AtomicBool) -> Result<(), String> {
    let t = Instant::now();
    while t.elapsed() < ENGINE_WARMUP {
        if cancel.load(Ordering::Relaxed) {
            return Ok(());
        }
        if let Ok(Some(_)) = child.try_wait() {
            return Err(engine_error(&fs::read_to_string(log_path(name)).unwrap_or_default()));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}

pub fn start_trial(paths: &Paths, s: &Strategy, game_filter: &GameFilter) -> anyhow::Result<Child> {
    let mut child = start_engine(paths, s, game_filter)?;
    match warm_up(&mut child, &s.name, &AtomicBool::new(false)) {
        Ok(()) => Ok(child),
        Err(e) => {
            stop_engine(Some(child));
            anyhow::bail!("стратегия не запустилась: {e}")
        }
    }
}

pub fn stop_trial(child: Child) {
    stop_engine(Some(child));
}

fn stop_engine(child: Option<Child>) {
    if let Some(mut c) = child {
        let _ = c.kill();
        let _ = c.wait();
    }
    kill_process("HolyZapret.exe");
    kill_process("winws.exe");
    std::thread::sleep(Duration::from_millis(400));
}

pub fn run(
    paths: Paths,
    strategies: Vec<Strategy>,
    targets: Vec<Target>,
    game_filter: GameFilter,
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
    if !cancel.load(Ordering::Relaxed) {
        emit(Event::Done(Outcome { name: "Без обхода".into(), path: None, error: None, checks: base }));
    }

    for (i, s) in strategies.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        emit(Event::Phase { index: i + 1, name: s.name.clone() });
        let mut outcome = Outcome { name: s.name.clone(), path: Some(s.path.clone()), error: None, checks: Vec::new() };
        match start_engine(&paths, s, &game_filter) {
            Ok(mut child) => match warm_up(&mut child, &s.name, &cancel) {
                Ok(()) => {
                    outcome.checks = run_suite(&targets, &cancel, &emit);
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
            outcome.checks = vec![Check { ok: false, frozen: false, ms: 0 }; targets.len()];
        }
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

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(judge(CheckKind::Tls13, true, "200 0 0.150").ok);
        assert_eq!(judge(CheckKind::Tls13, true, "200 0 0,150").ms, 150);
        assert!(!judge(CheckKind::Tls12, false, "000 0 5.0").ok);
        let f = judge(CheckKind::Freeze, false, "206 17408 6.0");
        assert!(!f.ok && f.frozen);
        let e = judge(CheckKind::Freeze, false, "000 0 6.0");
        assert!(!e.ok && !e.frozen);
        assert!(judge(CheckKind::Freeze, true, "206 262144 1.2").ok);
        assert!(judge(CheckKind::Freeze, false, "206 81506 8.0").ok);
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
        let c = |ok, ms| Check { ok, frozen: false, ms };
        let o = Outcome { name: "x".into(), path: None, error: None, checks: vec![c(true, 100), c(true, 300), c(false, 0), c(true, 200)] };
        assert_eq!(o.score(), (3, 4));
        assert_eq!(o.category(&targets, "B"), (1, 2));
        assert_eq!(o.avg_ms(), 200);
    }
}


