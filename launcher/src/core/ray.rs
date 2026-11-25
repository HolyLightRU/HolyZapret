use super::paths::Paths;
use super::service::{self, ServiceState};
use super::strategy::{self, Strategy};
use super::{kill_process, run_hidden};
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;

pub const SERVICE: &str = "rayoflight";
pub const EXE: &str = "RayOfLight.exe";
pub const LOG: &str = "rayoflight.log";
pub const LIVE: &str = "rayoflight-live.json";
pub const MEMORY: &str = "rayoflight-memory.tsv";
pub const AUTOLIST: &str = "rayoflight-autolist.txt";
pub const DEVICE: &str = "устройство ";
pub const HISTORY: usize = 31;

pub fn autolist_path(paths: &Paths) -> PathBuf {
    data_file(paths, AUTOLIST)
}

pub fn auto_sites(paths: &Paths) -> Vec<String> {
    std::fs::read_to_string(autolist_path(paths)).unwrap_or_default().lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect()
}

pub fn clear_autolist(paths: &Paths) {
    let _ = std::fs::remove_file(autolist_path(paths));
}
pub const EXPERIMENTS: [(&str, &str, &str); 9] = [
    ("general", "Как general", "Повторяет стратегию general один в один"),
    ("sni-split", "Разрез по имени сайта", "Режет ClientHello посередине имени сайта, а не после первого байта"),
    ("random", "Случайный рисунок", "Случайное имя в фейке, место разреза и число повторов в каждом соединении"),
    ("ttl", "Фейк с коротким TTL", "Фейк доходит до фильтра провайдера и пропадает по дороге, не доходя до сайта. Расстояние до сайта берётся из его ответа"),
    ("disorder", "Части в обратном порядке", "Сначала уходит конец ClientHello, потом начало"),
    ("badseq", "Фейк с неверным seq", "Фейк защищён неверным номером, работает и без TCP timestamps"),
    ("md5sig", "Фейк с подписью md5", "Фейк защищён подписью TCP MD5, работает и без TCP timestamps"),
    ("badsum", "Фейк с неверной суммой", "У фейка испорчена контрольная сумма: сайт его выбросит, а фильтр провайдера может не проверить"),
    ("adaptive", "Самоподстройка", "Если сайт два раза подряд ответил сбросом или тишиной, пробует другой приём и запоминает рабочий. Третьим шагом пробует пустить сайт без обхода: некоторые сайты обход ломает"),
];

pub const QUIC_EXPERIMENTS: [(&str, &str, &str); 5] = [
    (
        "adaptive",
        "QUIC: самоподстройка",
        "Сам выбирает приём для каждого сайта: как general, случайные фейки, без обхода. Если ничего не помогло, глушит QUIC, и браузер переходит на обычный HTTPS",
    ),
    ("general", "QUIC: как general", "11 фейков перед первым пакетом QUIC, как у обычного движка"),
    ("random", "QUIC: случайные фейки", "Фейки QUIC с разными именами сайтов и разного размера"),
    ("shuffle", "QUIC: перемешанный ClientHello", "Имя сайта режется на мелкие куски, и они перемешиваются внутри первого пакета"),
    ("block", "QUIC: глушить", "Первый пакет QUIC не уходит, и браузер переходит на обычный HTTPS"),
];

pub fn quic_label(name: &str) -> &'static str {
    if name == "none" {
        return "QUIC: без обхода";
    }
    QUIC_EXPERIMENTS.iter().find(|e| e.0 == name).map(|e| e.1).unwrap_or("На обычном движке")
}

pub const VIRTUAL_EXT: &str = "virtual";

pub fn virtual_strategies(paths: &Paths, all: &[Strategy]) -> Vec<Strategy> {
    if !super::TEST_BUILD {
        return Vec::new();
    }
    variants(paths, all)
}

pub fn quick_variant(paths: &Paths, all: &[Strategy]) -> Option<Strategy> {
    let id = format!("rayoflight:{}", super::settings::DEFAULT_EXPERIMENT);
    variants(paths, all).into_iter().find(|v| v.id == id)
}

fn variants(paths: &Paths, all: &[Strategy]) -> Vec<Strategy> {
    if exe(paths).is_none() {
        return Vec::new();
    }
    let Some(g) = all.iter().find(|s| s.id == "general") else { return Vec::new() };
    let lists = lists_arg(g, paths);
    let Some(tcp) = strategy::extract_args(g, paths, &super::gamefilter::GameFilter::default()).ok().and_then(|a| strip(&a, &lists)) else { return Vec::new() };
    let make = |name: String, label: &str| {
        let mut v = g.clone();
        v.id = format!("rayoflight:{name}");
        v.name = format!("RayOfLight · {label}");
        v.category = "RayOfLight".into();
        v.path = g.path.with_file_name(format!("rayoflight-{name}.{VIRTUAL_EXT}"));
        v.ray = Some((name, g.path.clone()));
        v
    };
    let mut out: Vec<Strategy> = EXPERIMENTS.iter().map(|(name, label, _)| make(name.to_string(), label)).collect();
    if strip_quic(&tcp, &lists).is_some() {
        out.extend(QUIC_EXPERIMENTS.iter().map(|(name, label, _)| make(format!("quic-{name}"), label)));
    }
    out
}

pub fn virtual_experiment(path: &std::path::Path) -> Option<String> {
    if path.extension().is_none_or(|e| e != VIRTUAL_EXT) {
        return None;
    }
    path.file_stem()?.to_str()?.strip_prefix("rayoflight-").map(str::to_string)
}

pub fn cli_args(paths: &Paths, s: &Strategy, experiment: &str, live: &std::path::Path) -> String {
    let (tcp, quic) = match experiment.strip_prefix("quic-") {
        Some(q) => ("general", Some(q)),
        None => (experiment, None),
    };
    let mut a = format!(
        "--bin \"{}\" --lists \"{}\" --experiment {tcp} --priority {} --live \"{}\" --live-ms {}",
        paths.bin().display(),
        strategy::lists_dir(s, paths).display(),
        -100,
        live.display(),
        PICK_LIVE_MS
    );
    if let Some(q) = quic {
        a.push_str(&format!(" --quic {q}"));
    }
    a
}

pub const PICK_LIVE_MS: u64 = 300;

pub const QUIC_BLOCK: &str = "quic-block";

pub fn tech_result(t: &TechLive) -> String {
    if t.name == QUIC_BLOCK { format!("заглушено {}", t.used) } else { format!("{} из {}", t.answered, t.answered + t.reset + t.silent) }
}

pub fn pick_summary(l: &Live) -> String {
    let blocked: u64 = l.techniques.iter().filter(|t| t.name == QUIC_BLOCK).map(|t| t.used).sum();
    let settled = l.answered + l.reset + l.silent + blocked;
    let mut s = format!(
        "RayOfLight: обработано {}, пропущено {}, ответ {}, сброс {}, тишина {}, без итога {}, без TCP timestamps {}, запасной приём {}",
        l.handled,
        l.skipped,
        l.answered,
        l.reset,
        l.silent,
        l.handled.saturating_sub(settled),
        l.no_ts,
        l.fallback
    );
    if l.techniques.len() > 1 {
        let t: Vec<String> = l.techniques.iter().map(|t| if t.name == QUIC_BLOCK { format!("{} заглушено {}", t.name, t.used) } else { format!("{} {} из {}", t.name, t.answered, t.used) }).collect();
        s.push_str(&format!("; приёмы: {}", t.join(", ")));
    }
    s
}

pub fn experiment_inline(name: &str) -> String {
    let label = experiment_label(name);
    if label.starts_with("QUIC") {
        return label.to_string();
    }
    let mut c = label.chars();
    c.next().map(|f| f.to_lowercase().chain(c).collect()).unwrap_or_default()
}

pub fn experiment_label(name: &str) -> &'static str {
    if let Some(q) = name.strip_prefix("quic-") {
        return quic_label(q);
    }
    if name == "none" {
        return "Без обхода";
    }
    EXPERIMENTS.iter().find(|e| e.0 == name).map(|e| e.1).unwrap_or("Как general")
}
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const SIGNATURE: [&str; 5] =
    ["--dpi-desync=fake,multisplit", "--dpi-desync-split-seqovl=681", "--dpi-desync-split-pos=1", "--dpi-desync-fooling=ts", "--dpi-desync-repeats=8"];

pub fn exe(paths: &Paths) -> Option<PathBuf> {
    let shipped = paths.bin().join(EXE);
    if shipped.is_file() {
        return Some(shipped);
    }
    if !super::TEST_BUILD {
        return None;
    }
    ["fast", "release"].iter().map(|p| paths.root.join("launcher").join("target").join(p).join(EXE)).find(|p| p.is_file())
}

fn data_file(paths: &Paths, name: &str) -> PathBuf {
    match std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
        Some(d) => {
            let dir = d.join("HolyZapret");
            let _ = std::fs::create_dir_all(&dir);
            dir.join(name)
        }
        None => paths.root.join(name),
    }
}

pub fn log_path(paths: &Paths) -> PathBuf {
    data_file(paths, LOG)
}

pub fn live_path(paths: &Paths) -> PathBuf {
    data_file(paths, LIVE)
}

pub fn memory_path(paths: &Paths) -> PathBuf {
    data_file(paths, MEMORY)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LiveItem {
    pub parts: u64,
    pub time: String,
    pub host: Option<String>,
    pub app: Option<String>,
    pub handled: bool,
    pub why: String,
    pub technique: Option<String>,
    pub outcome: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TechLive {
    pub name: String,
    pub used: u64,
    pub answered: u64,
    pub reset: u64,
    pub silent: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Live {
    pub running: bool,
    pub gateway: bool,
    pub history: Vec<(u64, u64, u64)>,
    pub cpu_x100: u64,
    pub mem: u64,
    pub time: String,
    pub mode: String,
    pub handled: u64,
    pub skipped: u64,
    pub no_ts: u64,
    pub fallback: u64,
    pub voice: u64,
    pub game_udp: u64,
    pub answered: u64,
    pub reset: u64,
    pub silent: u64,
    pub techniques: Vec<TechLive>,
    pub recent: Vec<LiveItem>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AppActivity {
    pub app: String,
    pub handled: usize,
    pub skipped: usize,
    pub answered: usize,
    pub failed: usize,
    pub hosts: Vec<String>,
}

pub fn outcome_label(o: &str) -> &'static str {
    match o {
        "answered" => "сервер ответил",
        "reset" => "пришёл сброс",
        "silent" => "нет ответа",
        _ => "",
    }
}

pub fn parse_live(text: &str) -> Option<Live> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    let s = |x: &serde_json::Value| x.as_str().map(str::to_string);
    let recent = v["recent"]
        .as_array()?
        .iter()
        .map(|i| LiveItem {
            parts: i["parts"].as_u64().unwrap_or(0),
            time: s(&i["time"]).unwrap_or_default(),
            host: s(&i["host"]),
            app: s(&i["app"]),
            handled: i["handled"].as_bool().unwrap_or(false),
            why: s(&i["why"]).unwrap_or_default(),
            technique: s(&i["technique"]),
            outcome: s(&i["outcome"]),
        })
        .collect();
    let n = |x: &serde_json::Value| x.as_u64().unwrap_or(0);
    let techniques = v["techniques"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|t| TechLive { name: s(&t["name"]).unwrap_or_default(), used: n(&t["used"]), answered: n(&t["answered"]), reset: n(&t["reset"]), silent: n(&t["silent"]) })
                .collect()
        })
        .unwrap_or_default();
    Some(Live {
        running: v["running"].as_bool().unwrap_or(false),
        gateway: v["gateway"].as_bool().unwrap_or(false),
        history: v["history"]
            .as_array()
            .map(|a| a.iter().filter_map(|r| Some((r.get(0)?.as_u64()?, r.get(1)?.as_u64()?, r.get(2)?.as_u64()?))).collect())
            .unwrap_or_default(),
        cpu_x100: (v["cpu"].as_f64().unwrap_or(0.0) * 100.0).round().max(0.0) as u64,
        mem: n(&v["mem"]),
        time: s(&v["time"]).unwrap_or_default(),
        mode: s(&v["mode"]).unwrap_or_else(|| "general".into()),
        handled: n(&v["handled"]),
        skipped: n(&v["skipped"]),
        no_ts: n(&v["no_ts"]),
        fallback: n(&v["fallback"]),
        voice: n(&v["voice"]),
        game_udp: n(&v["game_udp"]),
        answered: n(&v["answered"]),
        reset: n(&v["reset"]),
        silent: n(&v["silent"]),
        techniques,
        recent,
    })
}

pub fn wait_ready(paths: &Paths, timeout: std::time::Duration) -> Option<bool> {
    if !wanted() || !installed() {
        return None;
    }
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        if read_live(paths).is_some_and(|(l, age)| l.running && age < start.elapsed()) {
            return Some(true);
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    Some(false)
}

pub fn read_live(paths: &Paths) -> Option<(Live, std::time::Duration)> {
    let p = live_path(paths);
    let age = std::fs::metadata(&p).ok()?.modified().ok()?.elapsed().unwrap_or_default();
    Some((parse_live(&std::fs::read_to_string(&p).ok()?)?, age))
}

pub fn by_app(live: &Live) -> Vec<AppActivity> {
    let mut out: Vec<AppActivity> = Vec::new();
    for i in live.recent.iter().rev() {
        let app = i.app.clone().unwrap_or_else(|| "неизвестная программа".into());
        let pos = match out.iter().position(|a| a.app == app) {
            Some(p) => p,
            None => {
                out.push(AppActivity { app, ..Default::default() });
                out.len() - 1
            }
        };
        let a = &mut out[pos];
        if i.handled {
            a.handled += 1;
        } else {
            a.skipped += 1;
        }
        match i.outcome.as_deref() {
            Some("answered") => a.answered += 1,
            Some(_) => a.failed += 1,
            None => {}
        }
        if let Some(h) = &i.host {
            if a.hosts.len() < 3 && !a.hosts.contains(h) {
                a.hosts.push(h.clone());
            }
        }
    }
    out
}

pub fn lists_arg(s: &Strategy, paths: &Paths) -> String {
    format!("{}\\", strategy::lists_dir(s, paths).display())
}

fn ports<'a>(block: &'a str, key: &str) -> Option<&'a str> {
    block.split_whitespace().find_map(|t| t.strip_prefix(key))
}

fn has_443(p: &str) -> bool {
    p.split(',').any(|x| x.trim() == "443")
}

fn cut_profile(blocks: &[&str], idx: usize, key: &str) -> Option<String> {
    if blocks.len() < 2 {
        return None;
    }
    let mut out: Vec<String> = blocks.iter().map(|b| b.to_string()).collect();
    let current = ports(blocks[idx], key).unwrap_or("");
    let rest: Vec<&str> = current.split(',').map(str::trim).filter(|x| *x != "443" && !x.is_empty()).collect();
    if !rest.is_empty() {
        out[idx] = blocks[idx].replacen(&format!("{key}{current}"), &format!("{key}{}", rest.join(",")), 1);
        return Some(out.join(" --new "));
    }
    let removed = out.remove(idx);
    if idx == 0 {
        let wf: Vec<&str> = removed.split_whitespace().filter(|t| t.starts_with("--wf-")).collect();
        out[0] = format!("{} {}", wf.join(" "), out[0]);
    }
    Some(out.join(" --new "))
}

pub fn strip(args: &str, lists: &str) -> Option<String> {
    let general = format!("--hostlist=\"{lists}list-general.txt\"");
    let blocks: Vec<&str> = args.split(" --new ").collect();
    let idx = blocks.iter().position(|b| {
        ports(b, "--filter-tcp=").is_some_and(has_443) && b.contains(&general) && SIGNATURE.iter().all(|s| b.split_whitespace().any(|t| t == *s))
    })?;
    cut_profile(&blocks, idx, "--filter-tcp=")
}

pub fn strip_quic(args: &str, lists: &str) -> Option<String> {
    let general = format!("--hostlist=\"{lists}list-general.txt\"");
    let blocks: Vec<&str> = args.split(" --new ").collect();
    let idx = blocks.iter().position(|b| {
        ports(b, "--filter-udp=").is_some_and(has_443) && b.contains(&general) && b.split_whitespace().any(|t| t.starts_with("--dpi-desync-fake-quic="))
    })?;
    cut_profile(&blocks, idx, "--filter-udp=")
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Takeover {
    pub args: String,
    pub quic: bool,
    pub own: Vec<&'static str>,
    pub game_tcp: Option<String>,
    pub game_udp: Option<String>,
}

impl Takeover {
    pub fn cli(&self) -> String {
        let mut a = String::new();
        if !self.own.is_empty() {
            a.push_str(&format!(" --own {}", self.own.join(",")));
        }
        if let Some(p) = &self.game_tcp {
            a.push_str(&format!(" --game-tcp {p}"));
        }
        if let Some(p) = &self.game_udp {
            a.push_str(&format!(" --game-udp {p}"));
        }
        a
    }
}

fn drop_block(args: &str, idx: usize) -> String {
    let mut blocks: Vec<String> = args.split(" --new ").map(String::from).collect();
    let removed = blocks.remove(idx);
    let wf: Vec<&str> = removed.split_whitespace().filter(|t| t.starts_with("--wf-")).collect();
    if blocks.is_empty() {
        return wf.join(" ");
    }
    if idx == 0 && !wf.is_empty() {
        blocks[0] = format!("{} {}", wf.join(" "), blocks[0]);
    }
    blocks.join(" --new ")
}

fn has(b: &str, tok: &str) -> bool {
    b.split_whitespace().any(|t| t == tok)
}

fn has_file(b: &str, key: &str, file: &str) -> bool {
    b.split_whitespace().any(|t| t.starts_with(key) && t.contains(file))
}

fn game_ports(p: Option<&str>) -> Option<String> {
    p.filter(|v| *v != super::gamefilter::OFF_PORT).map(String::from)
}

pub fn takeover(args: &str, lists: &str, quic: bool) -> Option<Takeover> {
    let mut a = strip(args, lists)?;
    let mut t = Takeover::default();
    if quic {
        if let Some(q) = strip_quic(&a, lists) {
            a = q;
            t.quic = true;
        }
    }
    let general = format!("--hostlist=\"{lists}list-general.txt\"");
    let google = format!("--hostlist=\"{lists}list-google.txt\"");
    let ipset = format!("--ipset=\"{lists}ipset-all-user.txt\"");
    let sig = |b: &str| SIGNATURE.iter().all(|s| has(b, s));
    let sferum = "tls_clienthello_www_sferum_ru.bin";
    let tcp = |b: &str| ports(b, "--filter-tcp=").map(String::from);
    let udp = |b: &str| ports(b, "--filter-udp=").map(String::from);
    let quic_owned = t.quic;
    let rules: Vec<(&'static str, Box<dyn Fn(&str) -> bool + '_>)> = vec![
        ("http", Box::new(|b: &str| tcp(b).as_deref() == Some("80") && b.contains(&general) && sig(b) && has_file(b, "--dpi-desync-fake-http=", sferum))),
        ("google", Box::new(|b: &str| tcp(b).as_deref() == Some("443") && b.contains(&google) && has(b, "--ip-id=zero") && sig(b))),
        ("cf", Box::new(|b: &str| tcp(b).as_deref() == Some("2053,2083,2087,2096,8443") && has(b, "--hostlist-domains=discord.media") && sig(b))),
        (
            "voice",
            Box::new(|b: &str| {
                udp(b).as_deref() == Some("19294-19344,50000-50100")
                    && has(b, "--filter-l7=discord,stun")
                    && has(b, "--dpi-desync=fake")
                    && has(b, "--dpi-desync-repeats=6")
                    && has_file(b, "--dpi-desync-fake-discord=", "ACTIVE_DISCORD_UDP.bin")
                    && has_file(b, "--dpi-desync-fake-stun=", "ACTIVE_DISCORD_UDP.bin")
            }),
        ),
        (
            "myip-quic",
            Box::new(|b: &str| {
                quic_owned
                    && udp(b).as_deref() == Some("443")
                    && b.contains(&ipset)
                    && has(b, "--dpi-desync=fake")
                    && has(b, "--dpi-desync-repeats=11")
                    && has_file(b, "--dpi-desync-fake-quic=", "quic_initial_www_google_com.bin")
            }),
        ),
        ("myip-tcp", Box::new(|b: &str| tcp(b).as_deref() == Some("80,443,8443") && b.contains(&ipset) && sig(b) && has_file(b, "--dpi-desync-fake-http=", sferum))),
        (
            "game-tcp",
            Box::new(|b: &str| {
                tcp(b).is_some()
                    && b.contains(&ipset)
                    && has(b, "--dpi-desync-any-protocol=1")
                    && has(b, "--dpi-desync-cutoff=n4")
                    && sig(b)
                    && has_file(b, "--dpi-desync-fake-unknown=", sferum)
            }),
        ),
        (
            "game-udp",
            Box::new(|b: &str| {
                udp(b).is_some()
                    && b.contains(&ipset)
                    && has(b, "--dpi-desync=fake")
                    && has(b, "--dpi-desync-repeats=10")
                    && has(b, "--dpi-desync-any-protocol=1")
                    && has(b, "--dpi-desync-cutoff=n3")
                    && has_file(b, "--dpi-desync-fake-unknown-udp=", "ACTIVE_GAME_UDP.bin")
            }),
        ),
    ];
    for (name, pred) in &rules {
        let found = a.split(" --new ").position(|b| pred(b));
        if let Some(idx) = found {
            let block = a.split(" --new ").nth(idx).unwrap_or("").to_string();
            match *name {
                "game-tcp" => t.game_tcp = game_ports(tcp(&block).as_deref()),
                "game-udp" => t.game_udp = game_ports(udp(&block).as_deref()),
                _ => {}
            }
            a = drop_block(&a, idx);
            t.own.push(name);
        }
    }
    t.args = a;
    Some(t)
}

pub fn takeover_for(paths: &Paths, s: &Strategy) -> Option<Takeover> {
    let args = strategy::extract_args(s, paths, &super::gamefilter::load(paths)).ok()?;
    takeover(&args, &lists_arg(s, paths), !super::settings::ray_quic().is_empty())
}

pub fn wanted() -> bool {
    super::settings::ray_enabled()
}

pub fn applies(paths: &Paths, s: &Strategy, args: &str) -> bool {
    wanted() && exe(paths).is_some() && strip(args, &lists_arg(s, paths)).is_some()
}

pub fn command(paths: &Paths, s: &Strategy) -> Option<String> {
    let exe = exe(paths)?;
    let (all, skip) = super::settings::ray_apps();
    let mut cmd = format!(
        "\"{}\" --service --follow {} --bin \"{}\" --lists \"{}\" --log \"{}\"",
        exe.display(),
        service::SERVICE,
        paths.bin().display(),
        strategy::lists_dir(s, paths).display(),
        log_path(paths).display()
    );
    cmd.push_str(&format!(" --live \"{}\"", live_path(paths).display()));
    cmd.push_str(&format!(" --experiment {} --memory \"{}\"", super::settings::ray_experiment(), memory_path(paths).display()));
    if super::settings::ray_autolist() {
        cmd.push_str(&format!(" --autolist \"{}\"", autolist_path(paths).display()));
    }
    if super::settings::ray_gateway() {
        cmd.push_str(" --forward");
    }
    if let Some(t) = takeover_for(paths, s) {
        if t.quic {
            cmd.push_str(&format!(" --quic {}", super::settings::ray_quic()));
        }
        cmd.push_str(&t.cli());
    }
    for (flag, v) in [("--app-all", all), ("--app-skip", skip)] {
        let v: Vec<String> = v.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect();
        if !v.is_empty() {
            cmd.push_str(&format!(" {flag} \"{}\"", v.join(",")));
        }
    }
    Some(cmd)
}

pub fn remove() {
    run_hidden("net", &["stop", SERVICE]);
    run_hidden("sc", &["delete", SERVICE]);
    kill_process(EXE);
}

pub fn installed() -> bool {
    service::query(SERVICE) != ServiceState::NotInstalled
}

pub fn sync(paths: &Paths, s: &Strategy, active: bool) -> anyhow::Result<Option<String>> {
    if !active {
        if installed() {
            remove();
            return Ok(Some("RayOfLight выключен".into()));
        }
        return Ok(None);
    }
    let cmd = command(paths, s).ok_or_else(|| anyhow::anyhow!("нет {}", paths.bin().join(EXE).display()))?;
    remove();
    let raw = format!("create {SERVICE} binPath= \"{}\" DisplayName= \"RayOfLight\" start= auto", cmd.replace('"', "\\\""));
    let out = Command::new("sc").raw_arg(raw).creation_flags(CREATE_NO_WINDOW).output()?;
    if !out.status.success() {
        anyhow::bail!("sc create {SERVICE}: {}", String::from_utf8_lossy(&out.stdout));
    }
    run_hidden("sc", &["description", SERVICE, "RayOfLight: собственный движок HolyZapret"]);
    let (ok, o) = run_hidden("sc", &["start", SERVICE]);
    if !ok {
        anyhow::bail!("служба RayOfLight создана, но не запустилась: {o}");
    }
    Ok(Some("RayOfLight работает вместе со службой".into()))
}

pub fn report_lines(paths: &Paths) -> Vec<String> {
    let mut out = vec![format!(
        "Включён: {}, эксперимент: {} ({})",
        if super::settings::ray_enabled() { "да" } else { "нет" },
        experiment_label(&super::settings::ray_experiment()),
        super::settings::ray_experiment()
    )];
    let (all, skip) = super::settings::ray_apps();
    if !all.is_empty() || !skip.is_empty() {
        out.push(format!("Обходить целиком: {}; не трогать: {}", if all.is_empty() { "—" } else { &all }, if skip.is_empty() { "—" } else { &skip }));
    }
    match read_live(paths) {
        Some((l, age)) => {
            out.push(format!(
                "Данные движка {} с назад, {}: обработано {}, пропущено {}; итоги — ответ {}, сброс {}, тишина {}",
                age.as_secs(),
                if l.running { "работает" } else { "остановлен" },
                l.handled,
                l.skipped,
                l.answered,
                l.reset,
                l.silent
            ));
            for t in &l.techniques {
                out.push(format!("Приём {}: применён {}, ответ {}, сброс {}, тишина {}", t.name, t.used, t.answered, t.reset, t.silent));
            }
            out.push(format!("Голос Discord: фейки перед {} пакетами; игры UDP: {}", l.voice, l.game_udp));
            let line = |i: &LiveItem| {
                format!(
                    "{} {} · {} · {}{}{}",
                    i.time,
                    i.host.as_deref().unwrap_or("без имени"),
                    i.app.as_deref().unwrap_or("?"),
                    i.why,
                    i.technique.as_deref().map(|t| format!(" · {t}")).unwrap_or_default(),
                    i.outcome.as_deref().map(|o| format!(" · {}", outcome_label(o))).unwrap_or_default()
                ) + &if i.parts > 1 { format!(" · ClientHello в {} пакетах", i.parts) } else { String::new() }
            };
            let device = |i: &&LiveItem| i.app.as_deref().is_some_and(|a| a.starts_with(DEVICE));
            out.extend(l.recent.iter().rev().filter(|i| !device(i)).take(40).map(line));
            let gw: Vec<String> = l.recent.iter().rev().filter(device).take(60).map(line).collect();
            if !gw.is_empty() || l.gateway {
                out.push(format!("Шлюз: {}, решения по устройствам (последние {}):", if l.gateway { "включён в движке" } else { "в движке выключен" }, gw.len()));
                out.extend(gw.into_iter().map(|s| format!("  {s}")));
            }
        }
        None => out.push("Данных движка нет".into()),
    }
    let auto = auto_sites(paths);
    out.push(format!(
        "Автосписок: {}, сайтов {}{}",
        if super::settings::ray_autolist() { "включён" } else { "выключен" },
        auto.len(),
        if auto.is_empty() { String::new() } else { format!(": {}", auto.iter().take(30).cloned().collect::<Vec<_>>().join(", ")) }
    ));
    if let Ok(m) = std::fs::read_to_string(memory_path(paths)) {
        let rows: Vec<&str> = m.lines().take(40).collect();
        if !rows.is_empty() {
            out.push(format!("Самоподстройка запомнила сайтов: {}", m.lines().count()));
            out.extend(rows.iter().map(|r| format!("  {}", r.replace('\t', " · "))));
        }
    }
    let tail = log_tail(paths, 30);
    if !tail.is_empty() {
        out.push("Журнал движка, последние записи:".into());
        out.extend(tail.into_iter().map(|l| format!("  {l}")));
    }
    out
}

pub fn log_tail(paths: &Paths, n: usize) -> Vec<String> {
    let Ok(bytes) = std::fs::read(log_path(paths)) else { return Vec::new() };
    let text = String::from_utf8_lossy(&bytes);
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(n)..].iter().map(|l| l.to_string()).collect()
}

pub fn last_line(paths: &Paths) -> Option<String> {
    let text = std::fs::read_to_string(log_path(paths)).ok()?;
    text.lines().rev().find(|l| !l.trim().is_empty()).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    const L: &str = "C:\\HZ\\exp-list\\";

    fn general() -> String {
        format!(
            "--wf-tcp=80,443,12 --wf-udp=443,12 --filter-udp=443 --hostlist=\"{L}list-general.txt\" --dpi-desync=fake --dpi-desync-repeats=11 --dpi-desync-fake-quic=\"q.bin\" --new --filter-tcp=443 --hostlist=\"{L}list-google.txt\" --ip-id=zero --dpi-desync=fake,multisplit --dpi-desync-split-seqovl=681 --dpi-desync-split-pos=1 --dpi-desync-fooling=ts --dpi-desync-repeats=8 --new --filter-tcp=80,443 --hostlist=\"{L}list-general.txt\" --hostlist=\"{L}list-general-user.txt\" --dpi-desync=fake,multisplit --dpi-desync-split-seqovl=681 --dpi-desync-split-pos=1 --dpi-desync-fooling=ts --dpi-desync-repeats=8 --dpi-desync-fake-http=\"x.bin\" --new --filter-tcp=80,443,8443 --ipset=\"{L}ipset-all-user.txt\" --dpi-desync=fake"
        )
    }

    #[test]
    fn strips_only_the_general_list_profile() {
        let out = strip(&general(), L).unwrap();
        assert_eq!(out.matches(" --new ").count(), 3);
        assert!(out.contains("--hostlist=\"C:\\HZ\\exp-list\\list-google.txt\""));
        assert!(out.contains(" --new --filter-tcp=80 --hostlist=\"C:\\HZ\\exp-list\\list-general.txt\" --hostlist=\"C:\\HZ\\exp-list\\list-general-user.txt\" --dpi-desync=fake,multisplit"));
        assert!(!out.contains("--filter-tcp=80,443 --hostlist=\"C:\\HZ\\exp-list\\list-general.txt\""));
        assert!(out.starts_with("--wf-tcp=80,443,12"));
        assert!(out.contains("--filter-udp=443 --hostlist=\"C:\\HZ\\exp-list\\list-general.txt\""));
    }

    #[test]
    fn reads_live_file_and_groups_by_app() {
        let text = r#"{"running":true,"time":"12:00:05","mode":"adaptive","handled":2,"skipped":1,"no_ts":0,"flushed":0,"flows":3,"answered":1,"reset":1,"silent":0,"techniques":[{"name":"general","used":2,"answered":1,"reset":1,"silent":0}],"recent":[
            {"time":"12:00:01","host":"www.youtube.com","app":"chrome.exe","pid":1,"handled":true,"why":"сайт в списке","technique":"general","outcome":"answered"},
            {"time":"12:00:02","host":"example.org","app":"chrome.exe","pid":1,"handled":false,"why":"сайта нет в списке"},
            {"time":"12:00:03","host":"discord.com","app":"discord.exe","pid":2,"handled":true,"why":"вся программа"},
            {"time":"12:00:04","host":null,"app":null,"pid":null,"handled":false,"why":"сайта нет в списке"}]}"#;
        let l = parse_live(text).unwrap();
        assert!(l.running && l.handled == 2 && l.recent.len() == 4);
        assert_eq!((l.mode.as_str(), l.answered, l.reset, l.techniques.len()), ("adaptive", 1, 1, 1));
        assert_eq!(l.recent[0].outcome.as_deref(), Some("answered"));
        assert_eq!(experiment_label("sni-split"), "Разрез по имени сайта");
        let apps = by_app(&l);
        assert_eq!(apps.iter().map(|a| a.app.as_str()).collect::<Vec<_>>(), vec!["неизвестная программа", "discord.exe", "chrome.exe"]);
        let chrome = &apps[2];
        assert_eq!((chrome.handled, chrome.skipped, chrome.answered, chrome.hosts.clone()), (1, 1, 1, vec!["example.org".to_string(), "www.youtube.com".to_string()]));
        assert_eq!(parse_live("not json"), None);
        let p = parse_live(r#"{"running":true,"mode":"adaptive","handled":5,"skipped":2,"no_ts":1,"fallback":1,"answered":3,"reset":1,"silent":0,"techniques":[{"name":"general","used":4,"answered":3,"reset":1,"silent":0},{"name":"sni-split","used":1,"answered":0,"reset":0,"silent":0}],"recent":[]}"#).unwrap();
        assert_eq!(
            pick_summary(&p),
            "RayOfLight: обработано 5, пропущено 2, ответ 3, сброс 1, тишина 0, без итога 1, без TCP timestamps 1, запасной приём 1; приёмы: general 3 из 4, sni-split 0 из 1"
        );
        let b = parse_live(r#"{"handled":4,"skipped":0,"answered":1,"reset":0,"silent":0,"techniques":[{"name":"general","used":1,"answered":1,"reset":0,"silent":0},{"name":"quic-block","used":3,"answered":0,"reset":0,"silent":0}],"recent":[]}"#).unwrap();
        assert!(pick_summary(&b).contains("без итога 0") && pick_summary(&b).ends_with("приёмы: general 1 из 1, quic-block заглушено 3"));
        assert_eq!(tech_result(&b.techniques[1]), "заглушено 3");
    }

    #[test]
    fn takes_over_every_general_profile() {
        let root = PathBuf::from("..").canonicalize().unwrap();
        if !root.join("bin").join("HolyZapret.exe").exists() {
            return;
        }
        let p = Paths { root };
        let all = strategy::discover(&p);
        let g = all.iter().find(|s| s.id == "general").unwrap();
        let lists = lists_arg(g, &p);
        let args = strategy::extract_args(g, &p, &crate::core::gamefilter::GameFilter::all()).unwrap();
        let t = takeover(&args, &lists, true).unwrap();
        assert_eq!(t.own, vec!["http", "google", "cf", "voice", "myip-quic", "myip-tcp", "game-tcp", "game-udp"]);
        assert!(t.quic);
        assert!(!t.args.contains(" --new ") && !t.args.contains("--filter-") && t.args.starts_with("--wf-tcp="), "{}", t.args);
        assert_eq!((t.game_tcp.as_deref(), t.game_udp.as_deref()), (Some("1024-65535"), Some("1024-65535")));
        assert!(t.cli().starts_with(" --own http,google,cf,voice,myip-quic,myip-tcp,game-tcp,game-udp --game-tcp 1024-65535"));
        let plain = takeover(&args, &lists, false).unwrap();
        assert!(!plain.quic && !plain.own.contains(&"myip-quic"));
        assert_eq!(plain.args.matches("--filter-udp=443 ").count(), 2);
        let off = strategy::extract_args(g, &p, &crate::core::gamefilter::GameFilter::default()).unwrap();
        let t = takeover(&off, &lists, false).unwrap();
        assert!(t.own.contains(&"game-tcp") && t.game_tcp.is_none() && t.game_udp.is_none());
        let changed = args.replace("--dpi-desync-repeats=6", "--dpi-desync-repeats=5");
        let t = takeover(&changed, &lists, false).unwrap();
        assert!(!t.own.contains(&"voice") && t.args.contains("--filter-l7=discord,stun"));
        assert_eq!(drop_block("--wf-tcp=1 --filter-tcp=1 --x", 0), "--wf-tcp=1");
    }

    #[test]
    fn strips_the_quic_list_profile() {
        let tcp = strip(&general(), L).unwrap();
        let out = strip_quic(&tcp, L).unwrap();
        assert!(!out.contains("--filter-udp=443 --hostlist=\"C:\\HZ\\exp-list\\list-general.txt\""));
        assert!(out.starts_with("--wf-tcp=80,443,12 --wf-udp=443,12 --filter-tcp=443"));
        assert_eq!(strip_quic(&tcp.replace("--dpi-desync-repeats=11", "--dpi-desync-repeats=11 --dpi-desync-fake-quic=\"q.bin\"").replace("--filter-udp=443 ", "--filter-udp=443,50000 "), L).map(|s| s.contains("--filter-udp=50000 --hostlist")), Some(true));
        assert_eq!(experiment_label("quic-shuffle"), "QUIC: перемешанный ClientHello");
        assert_eq!((experiment_inline("sni-split"), experiment_inline("quic-block")), ("разрез по имени сайта".to_string(), "QUIC: глушить".to_string()));
        assert_eq!(quic_label(""), "На обычном движке");
    }

    #[test]
    fn virtual_paths_round_trip() {
        let p = std::path::Path::new("C:\\HZ\\scripts\\rayoflight-sni-split.virtual");
        assert_eq!(virtual_experiment(p).as_deref(), Some("sni-split"));
        assert_eq!(virtual_experiment(std::path::Path::new("C:\\HZ\\scripts\\general.bat")), None);
        assert_eq!(virtual_experiment(std::path::Path::new("C:\\HZ\\other.virtual")), None);
    }

    #[test]
    fn refuses_other_strategies() {
        let alt = general().replace("--dpi-desync-repeats=8 --dpi-desync-fake-http", "--dpi-desync-repeats=6 --dpi-desync-fake-http");
        assert_eq!(strip(&alt, L), None);
        assert_eq!(strip(&general(), "D:\\other\\"), None);
        let first = format!("--wf-tcp=443 --filter-tcp=443 --hostlist=\"{L}list-general.txt\" --dpi-desync=fake,multisplit --dpi-desync-split-seqovl=681 --dpi-desync-split-pos=1 --dpi-desync-fooling=ts --dpi-desync-repeats=8 --new --filter-udp=443 --dpi-desync=fake");
        assert_eq!(strip(&first, L).unwrap(), "--wf-tcp=443 --filter-udp=443 --dpi-desync=fake");
        let only = first.split(" --new ").next().unwrap().to_string();
        assert_eq!(strip(&only, L), None);
    }
}
