use super::paths::Paths;
use super::service::{self, ServiceState};
use super::strategy::{self, Strategy};
use super::{kill_process, run_hidden};
use std::path::PathBuf;

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
pub const EXPERIMENTS: [(&str, &str, &str); 20] = [
    ("general", "Как general", "Основной приём: 8 фейков перед ClientHello и разрез после первого байта"),
    ("sni-split", "Разрез по имени сайта", "Режет ClientHello посередине имени сайта, а не после первого байта"),
    ("overlap", "Наложение без фейка", "Без фейков: начало ClientHello уходит с наложенным чужим ClientHello (узор 4pda.to), как в ALT 3"),
    ("fake-only", "Только фейки", "6 фейков с неверным seq и именем www.microsoft.com, а ClientHello уходит целиком, без разреза, как в ALT 2"),
    ("random", "Случайный рисунок", "Случайное имя в фейке, место разреза и число повторов в каждом соединении"),
    ("ttl", "Фейк с коротким TTL", "Фейк доходит до фильтра провайдера и пропадает по дороге, не доходя до сайта. Расстояние до сайта берётся из его ответа"),
    ("disorder", "Части в обратном порядке", "Сначала уходит конец ClientHello, потом начало"),
    ("badseq", "Фейк с неверным seq", "Фейк защищён неверным номером, работает и без TCP timestamps"),
    ("md5sig", "Фейк с подписью md5", "Фейк защищён подписью TCP MD5, работает и без TCP timestamps"),
    ("badsum", "Фейк с неверной суммой", "У фейка испорчена контрольная сумма: сайт его выбросит, а фильтр провайдера может не проверить"),
    ("sni-vk", "Фейк с именем vk.com", "Как general, но в фейке имя vk.com, как в R08"),
    ("sni-fonts", "Фейк с именем fonts.google.com", "Как general, но в фейке имя fonts.google.com, как в R12"),
    ("sni-max", "Фейк с именем max.ru", "Как general, но в фейке имя max.ru: фильтр провайдера может пропускать его в первую очередь"),
    ("fakedsplit", "Фейк и ложные части", "Фейк с неверным seq, а каждая часть ClientHello окружена ложными копиями из нулей, как в R11"),
    ("fakedsplit-ttl", "Ложные части с коротким TTL", "Без общего фейка: каждая часть ClientHello окружена ложными копиями с неверным seq и коротким TTL, как в ALT 1"),
    ("overlap-max", "Наложение с узором max.ru", "Без фейков: наложение 681 байт с узором max.ru, как в MAX"),
    ("overlap-2", "Наложение 652 после 2 байт", "Без фейков: разрез после второго байта и наложение 652 байт с узором max.ru, как в ALT 5"),
    ("overlap-sld", "Наложение и разрез домена", "Без фейков: наложение с узором max.ru и второй разрез посередине имени сайта, как у YouTube в ALT 5"),
    ("fake-ts", "Только фейки ts", "6 фейков с неверной временной меткой, а ClientHello уходит целиком, как в ALT 4"),
    ("adaptive", "Самоподстройка", "Если сайт два раза подряд ответил сбросом или тишиной, пробует другой приём и запоминает рабочий. Третьим шагом пробует фейк с именем max.ru, потом пустить сайт без обхода: некоторые сайты обход ломает"),
];

pub const EXTRA: [&str; 9] = ["sni-vk", "sni-fonts", "overlap", "fake-only", "fakedsplit", "fakedsplit-ttl", "overlap-2", "overlap-sld", "fake-ts"];

pub const QUIC_EXPERIMENTS: [(&str, &str, &str); 5] = [
    (
        "adaptive",
        "QUIC: самоподстройка",
        "Сам выбирает приём для каждого сайта: как general, случайные фейки, без обхода. Если ничего не помогло, глушит QUIC, и браузер переходит на обычный HTTPS",
    ),
    ("general", "QUIC: как general", "11 фейков перед первым пакетом QUIC"),
    ("random", "QUIC: случайные фейки", "Фейки QUIC с разными именами сайтов и разного размера"),
    ("shuffle", "QUIC: перемешанный ClientHello", "Имя сайта режется на мелкие куски, и они перемешиваются внутри первого пакета"),
    ("block", "QUIC: глушить", "Первый пакет QUIC не уходит, и браузер переходит на обычный HTTPS"),
];

pub fn quic_label(name: &str) -> &'static str {
    if name == "none" {
        return "QUIC: без обхода";
    }
    QUIC_EXPERIMENTS.iter().find(|e| e.0 == name).map(|e| e.1).unwrap_or("QUIC: как general")
}

pub const VIRTUAL_EXT: &str = "virtual";
pub const DEFAULT_ID: &str = "rayoflight:adaptive";

pub fn variants(paths: &Paths) -> Vec<Strategy> {
    let Some(engine) = exe(paths) else { return Vec::new() };
    let dir = strategy::scripts_dir(paths);
    let make = |name: String, label: &str| {
        let path = dir.join(format!("rayoflight-{name}.{VIRTUAL_EXT}"));
        Strategy {
            id: format!("rayoflight:{name}"),
            name: if name == "adaptive" { "RayOfLight · Самоподстройка".into() } else { format!("RayOfLight · {label}") },
            path: path.clone(),
            engine: engine.clone(),
            bin: paths.bin(),
            lists: paths.exp_lists(),
            ray: Some((name, path)),
        }
    };
    let mut out = vec![make("adaptive".into(), "Самоподстройка")];
    out.extend(EXPERIMENTS.iter().filter(|e| e.0 != "adaptive").map(|(name, label, _)| make(name.to_string(), label)));
    out.extend(QUIC_EXPERIMENTS.iter().filter(|e| e.0 != "adaptive").map(|(name, label, _)| make(format!("quic-{name}"), label)));
    out
}

pub fn virtual_experiment(path: &std::path::Path) -> Option<String> {
    if path.extension().is_none_or(|e| e != VIRTUAL_EXT) {
        return None;
    }
    path.file_stem()?.to_str()?.strip_prefix("rayoflight-").map(str::to_string)
}

pub fn start_technique() -> Option<String> {
    let s = super::settings::ray_start();
    (!s.is_empty() && rayoflight::desync::technique(&s).is_some()).then_some(s)
}

pub fn tcp_experiment() -> String {
    let e = super::settings::ray_experiment();
    match start_technique() {
        Some(s) if e == "adaptive" => format!("adaptive:{s}"),
        _ => e,
    }
}

pub fn install_label(s: &Strategy) -> String {
    match s.ray.as_ref().map(|r| r.0.as_str()) {
        Some(n) if n.starts_with("quic-") => format!("RayOfLight · Самоподстройка · {}", quic_label(&n[5..])),
        Some(n) if n != "adaptive" => format!("RayOfLight · Самоподстройка, первым шагом «{}»", experiment_label(n)),
        _ => s.name.clone(),
    }
}

pub fn migrate_fixed_technique() -> Option<String> {
    if !super::settings::take_start_migration() {
        return None;
    }
    let e = super::settings::ray_experiment();
    if e == "adaptive" || rayoflight::desync::technique(&e).is_none() {
        return None;
    }
    super::settings::set_ray_start(&e);
    super::settings::set_ray_experiment("adaptive");
    Some(e)
}

pub fn split_experiment(experiment: &str) -> (&str, &str) {
    match experiment.strip_prefix("quic-") {
        Some(q) => ("general", q),
        None if experiment == "adaptive" => ("adaptive", "adaptive"),
        None => (experiment, "general"),
    }
}

pub fn engine_args(paths: &Paths, tcp: &str, quic: &str, gf: &super::gamefilter::GameFilter) -> String {
    let mut own = vec!["http", "google", "cf", "voice", "myip-tcp"];
    if !quic.is_empty() {
        own.push("myip-quic");
    }
    let mut games = String::new();
    let off = super::gamefilter::OFF_PORT;
    if gf.enabled() && gf.tcp_value() != off {
        own.push("game-tcp");
        games.push_str(&format!(" --game-tcp {}", gf.tcp_value()));
    }
    if gf.enabled() && gf.udp_value() != off {
        own.push("game-udp");
        games.push_str(&format!(" --game-udp {}", gf.udp_value()));
    }
    let mut a = format!("--bin \"{}\" --lists \"{}\" --experiment {tcp}", paths.bin().display(), paths.exp_lists().display());
    if !quic.is_empty() {
        a.push_str(&format!(" --quic {quic}"));
    }
    a.push_str(&format!(" --own {}{games}", own.join(",")));
    a
}

const PROFILES: &str = include_str!("../../assets/profiles.txt");

pub fn profile_args(paths: &Paths, gf: &super::gamefilter::GameFilter) -> String {
    let bin = format!("{}\\", paths.bin().display());
    let lists = format!("{}\\", paths.exp_lists().display());
    let args = PROFILES.trim().replace("%BIN%", &bin).replace("%LISTS%", &lists);
    let args = strategy::apply_game_filter(&args, gf);
    super::lists::inject_user_lists(args.trim(), &lists)
}

pub fn cli_args(paths: &Paths, experiment: &str, live: &std::path::Path, gf: &super::gamefilter::GameFilter) -> String {
    let (tcp, quic) = split_experiment(experiment);
    let dns = if super::settings::ray_dns() { " --dns" } else { "" };
    format!("{} --priority -100 --live \"{}\" --live-ms {}{dns}", engine_args(paths, tcp, quic, gf), live.display(), PICK_LIVE_MS)
}

pub const PICK_LIVE_MS: u64 = 300;

pub const QUIC_BLOCK: &str = "quic-block";

pub fn tech_result(t: &TechLive) -> String {
    if t.name == QUIC_BLOCK { format!("заглушено {}", t.used) } else { format!("{} из {}", t.answered, t.answered + t.reset + t.silent + t.frozen) }
}

pub fn pick_summary(l: &Live) -> String {
    let blocked: u64 = l.techniques.iter().filter(|t| t.name == QUIC_BLOCK).map(|t| t.used).sum();
    let settled = l.answered + l.reset + l.silent + l.frozen + blocked;
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
    match name {
        "none" | "udp-none" => return "Без обхода",
        "udp-fake" => return "Фейки как в general",
        "udp-random" => return "Случайные фейки",
        "udp-quic" => return "Фейки QUIC",
        "warp" => return "Через WARP",
        _ => {}
    }
    EXPERIMENTS.iter().find(|e| e.0 == name).map(|e| e.1).unwrap_or("Как general")
}
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

pub fn next_techniques(paths: &Paths) -> std::collections::HashMap<String, String> {
    let text = std::fs::read_to_string(memory_path(paths)).unwrap_or_default();
    parse_memory(&text)
}

pub fn parse_memory(text: &str) -> std::collections::HashMap<String, String> {
    text.lines()
        .filter_map(|l| {
            let mut f = l.split('\t');
            let (host, tech) = (f.next()?, f.next()?);
            (!host.is_empty() && !tech.starts_with("quic-")).then(|| (host.to_string(), tech.to_string()))
        })
        .collect()
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
    pub frozen: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Live {
    pub running: bool,
    pub gateway: bool,
    pub history: Vec<(u64, u64, u64, u64)>,
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
    pub frozen: u64,
    pub techniques: Vec<TechLive>,
    pub recent: Vec<LiveItem>,
    pub warp: Option<WarpLive>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WarpLive {
    pub ready: bool,
    pub wanted: bool,
    pub active: u64,
    pub ips: u64,
    pub done: u64,
    pub answered: u64,
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
        "frozen" => "сервер оборвал загрузку",
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
                .map(|t| TechLive { name: s(&t["name"]).unwrap_or_default(), used: n(&t["used"]), answered: n(&t["answered"]), reset: n(&t["reset"]), silent: n(&t["silent"]), frozen: n(&t["frozen"]) })
                .collect()
        })
        .unwrap_or_default();
    Some(Live {
        running: v["running"].as_bool().unwrap_or(false),
        gateway: v["gateway"].as_bool().unwrap_or(false),
        history: v["history"]
            .as_array()
            .map(|a| a.iter().filter_map(|r| Some((r.get(0)?.as_u64()?, r.get(1)?.as_u64()?, r.get(2)?.as_u64()?, r.get(3).and_then(|x| x.as_u64()).unwrap_or(0)))).collect())
            .unwrap_or_default(),
        cpu_x100: (v["cpu"].as_f64().unwrap_or(0.0) * 100.0).round().max(0.0) as u64,
        mem: n(&v["mem"]),
        warp: v["warp"].is_object().then(|| WarpLive {
            ready: v["warp"]["ready"].as_bool().unwrap_or(false),
            wanted: v["warp"]["wanted"].as_bool().unwrap_or(false),
            active: n(&v["warp"]["active"]),
            ips: n(&v["warp"]["ips"]),
            done: n(&v["warp"]["done"]),
            answered: n(&v["warp"]["answered"]),
        }),
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
        frozen: n(&v["frozen"]),
        techniques,
        recent,
    })
}

pub fn wait_ready(paths: &Paths, timeout: std::time::Duration) -> Option<bool> {
    if !wanted() || service::query(service::SERVICE) == ServiceState::NotInstalled {
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
        if i.app.as_deref() == Some(rayoflight::flows::BG_APP) {
            continue;
        }
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

pub fn wanted() -> bool {
    super::settings::ray_enabled()
}

pub fn command(paths: &Paths) -> Option<String> {
    let exe = exe(paths)?;
    let (all, skip) = super::settings::ray_apps();
    let experiment = super::settings::ray_experiment();
    let quic = match super::settings::ray_quic() {
        q if q.is_empty() => "general".to_string(),
        q => q,
    };
    let mut cmd = format!(
        "\"{}\" --service --log \"{}\" {}",
        exe.display(),
        log_path(paths).display(),
        engine_args(paths, &tcp_experiment(), &quic, &super::gamefilter::load(paths))
    );
    cmd.push_str(&format!(" --live \"{}\" --memory \"{}\"", live_path(paths).display(), memory_path(paths).display()));
    if super::settings::ray_dns() {
        cmd.push_str(" --dns");
    }
    if super::settings::ray_autolist() {
        cmd.push_str(&format!(" --autolist \"{}\"", autolist_path(paths).display()));
    }
    if super::settings::ray_gateway() {
        cmd.push_str(" --forward");
    }
    if super::settings::ray_warp() && experiment == "adaptive" {
        cmd.push_str(&format!(" --warp {}", super::warp::PROXY_PORT));
        for h in super::settings::ray_warp_force() {
            cmd.push_str(&format!(" --warp-force {h}"));
        }
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

pub fn report_lines(paths: &Paths) -> Vec<String> {
    let mut out = vec![format!(
        "Приём: {} ({}), QUIC: {}",
        experiment_label(&super::settings::ray_experiment()),
        super::settings::ray_experiment(),
        quic_label(&super::settings::ray_quic())
    )];
    let (all, skip) = super::settings::ray_apps();
    if !all.is_empty() || !skip.is_empty() {
        out.push(format!("Обходить целиком: {}; не трогать: {}", if all.is_empty() { "—" } else { &all }, if skip.is_empty() { "—" } else { &skip }));
    }
    match read_live(paths) {
        Some((l, age)) => {
            out.push(format!(
                "Данные движка {} с назад, {}: обработано {}, пропущено {}; итоги — ответ {}, сброс {}, тишина {}, заморозка {}",
                age.as_secs(),
                if l.running { "работает" } else { "остановлен" },
                l.handled,
                l.skipped,
                l.answered,
                l.reset,
                l.silent,
                l.frozen
            ));
            for t in &l.techniques {
                out.push(format!("Приём {}: применён {}, ответ {}, сброс {}, тишина {}, заморозка {}", t.name, t.used, t.answered, t.reset, t.silent, t.frozen));
            }
            out.push(format!("Голос Discord: фейки перед {} пакетами; игры UDP: фейки перед {} потоками", l.voice, l.game_udp));
            match &l.warp {
                Some(w) => out.push(format!(
                    "WARP как последний способ: прокси WARP {}, нужен {}, адресов через WARP {}, соединений сейчас {}, всего {}, ответили {}",
                    if w.ready { "доступен" } else { "недоступен" },
                    if w.wanted { "да" } else { "нет" },
                    w.ips,
                    w.active,
                    w.done,
                    w.answered
                )),
                None => out.push(format!("WARP как последний способ: {}", if super::settings::ray_warp() { "включён, но движок запущен без него" } else { "выключен" })),
            }
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
            let game = |i: &&LiveItem| i.host.as_deref().is_some_and(|h| h.starts_with(rayoflight::flows::UDP_KEY));
            out.extend(squash(l.recent.iter().rev().filter(|i| !device(i) && !game(i)).map(line), 40));
            let games = squash(l.recent.iter().rev().filter(game).map(line), 40);
            if !games.is_empty() {
                out.push(format!("Игры (UDP), последние решения ({}):", games.len()));
                out.extend(games.into_iter().map(|s| format!("  {s}")));
            }
            let gw: Vec<String> = l.recent.iter().rev().filter(device).take(60).map(line).collect();
            if !gw.is_empty() || l.gateway {
                out.push(format!("Шлюз: {}, решения по устройствам (последние {}):", if l.gateway { "включён в движке" } else { "в движке выключен" }, gw.len()));
                out.extend(gw.into_iter().map(|s| format!("  {s}")));
            }
        }
        None => out.push("Данных движка нет".into()),
    }
    out.push(format!("Свой DNS для сайтов из списков: {}", if super::settings::ray_dns() { "включён" } else { "выключен" }));
    let auto = auto_sites(paths);
    out.push(format!(
        "Автосписок: {}, сайтов {}{}",
        if super::settings::ray_autolist() { "включён" } else { "выключен" },
        auto.len(),
        if auto.is_empty() { String::new() } else { format!(": {}", auto.iter().take(30).cloned().collect::<Vec<_>>().join(", ")) }
    ));
    if let Ok(m) = std::fs::read_to_string(memory_path(paths)) {
        let (games, mut sites): (Vec<&str>, Vec<&str>) = m.lines().filter(|l| !l.trim().is_empty()).partition(|l| l.starts_with(rayoflight::flows::UDP_KEY));
        sites.sort_by_key(|r| memory_is_plain(r));
        if !sites.is_empty() {
            out.push(format!("Самоподстройка запомнила сайтов: {} (сначала со сбоями и особым приёмом)", sites.len()));
            out.extend(sites.iter().take(60).map(|r| format!("  {}", r.replace('\t', " · "))));
        }
        if !games.is_empty() {
            out.push(format!("Игры (UDP), самоподстройка запомнила программ: {}", games.len()));
            out.extend(games.iter().take(40).map(|r| format!("  {}", r.replace('\t', " · "))));
        }
    }
    let tail = log_tail(paths, 30);
    if !tail.is_empty() {
        out.push("Журнал движка, последние записи:".into());
        out.extend(tail.into_iter().map(|l| format!("  {l}")));
    }
    out
}

fn squash(lines: impl Iterator<Item = String>, limit: usize) -> Vec<String> {
    let mut out: Vec<(String, String, usize)> = Vec::new();
    for l in lines {
        let rest = l.split_once(' ').map(|(_, r)| r.to_string()).unwrap_or_else(|| l.clone());
        if let Some(last) = out.last_mut().filter(|last| last.1 == rest) {
            last.2 += 1;
            continue;
        }
        if out.len() >= limit {
            break;
        }
        out.push((l, rest, 1));
    }
    out.into_iter().map(|(l, _, n)| if n > 1 { format!("{l} (ещё {} таких же)", n - 1) } else { l }).collect()
}

fn memory_is_plain(row: &str) -> bool {
    let f: Vec<&str> = row.split('\t').collect();
    let plain_tech = f.get(1).is_some_and(|t| matches!(*t, "general" | "quic-general"));
    let no_fails = f.get(3).is_none_or(|n| n.trim() == "0");
    plain_tech && no_fails
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

    #[test]
    fn picked_technique_is_installed_as_the_first_step_of_self_tuning() {
        let v = |name: &str| Strategy {
            id: format!("rayoflight:{name}"),
            name: format!("RayOfLight · {name}"),
            path: PathBuf::new(),
            engine: PathBuf::new(),
            bin: PathBuf::new(),
            lists: PathBuf::new(),
            ray: Some((name.to_string(), PathBuf::new())),
        };
        assert_eq!(install_label(&v("sni-max")), "RayOfLight · Самоподстройка, первым шагом «Фейк с именем max.ru»");
        assert_eq!(install_label(&v("quic-random")), format!("RayOfLight · Самоподстройка · {}", quic_label("random")));
        assert_eq!(install_label(&v("adaptive")), "RayOfLight · adaptive");
    }

    #[test]
    fn every_listed_technique_exists_and_extras_stay_out_of_the_rotation() {
        for (name, _, _) in EXPERIMENTS.iter().filter(|e| e.0 != "adaptive") {
            assert!(rayoflight::desync::technique(name).is_some(), "{name}");
        }
        let Some(rayoflight::flows::Mode::Adaptive(list)) = rayoflight::flows::Mode::parse("adaptive") else { panic!() };
        for e in EXTRA {
            assert!(EXPERIMENTS.iter().any(|x| x.0 == e), "{e}");
            assert!(!list.iter().any(|t| t.name == e), "{e}");
            assert!(!super::super::rules::TECHNIQUES.contains(&e), "{e}");
        }
    }

    #[test]
    fn reads_live_file_and_groups_by_app() {
        let text = r#"{"running":true,"time":"12:00:05","mode":"adaptive","handled":2,"skipped":1,"no_ts":0,"flushed":0,"flows":3,"answered":1,"reset":1,"silent":0,"techniques":[{"name":"general","used":2,"answered":1,"reset":1,"silent":0}],"recent":[
            {"time":"12:00:01","host":"www.youtube.com","app":"chrome.exe","pid":1,"handled":true,"why":"сайт в списке","technique":"general","outcome":"answered"},
            {"time":"12:00:02","host":"example.org","app":"chrome.exe","pid":1,"handled":false,"why":"сайта нет в списке"},
            {"time":"12:00:03","host":"discord.com","app":"discord.exe","pid":2,"handled":true,"why":"вся программа"},
            {"time":"12:00:04","host":null,"app":null,"pid":null,"handled":false,"why":"сайта нет в списке"},
            {"time":"12:00:05","host":"x.com","app":"проверка самоподстройки","pid":3,"handled":true,"why":"проверка самоподстройки","technique":"sni-split","outcome":"answered"}]}"#;
        let l = parse_live(text).unwrap();
        assert!(l.running && l.handled == 2 && l.recent.len() == 5);
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
    fn virtual_paths_round_trip() {
        let p = std::path::Path::new("C:\\HZ\\scripts\\rayoflight-sni-split.virtual");
        assert_eq!(virtual_experiment(p).as_deref(), Some("sni-split"));
        assert_eq!(virtual_experiment(std::path::Path::new("C:\\HZ\\scripts\\general.bat")), None);
        assert_eq!(virtual_experiment(std::path::Path::new("C:\\HZ\\other.virtual")), None);
    }

}

#[cfg(test)]
mod report_tests {
    use super::{memory_is_plain, squash};

    #[test]
    fn repeated_decisions_are_squashed() {
        let lines = ["10:00:03 a · steam.exe · нет", "10:00:02 a · steam.exe · нет", "10:00:01 a · steam.exe · нет", "10:00:00 b · x · да", "09:59:59 c · x · да"];
        let got = squash(lines.iter().map(|s| s.to_string()), 2);
        assert_eq!(got, vec!["10:00:03 a · steam.exe · нет (ещё 2 таких же)".to_string(), "10:00:00 b · x · да".to_string()]);
    }

    #[test]
    fn memory_rows_with_trouble_go_first() {
        let mut rows = vec!["a.com	general	5	0", "b.com	none	1	0", "c.com	general	0	3", "d.com	quic-general	2	0"];
        rows.sort_by_key(|r| memory_is_plain(r));
        assert_eq!(rows.iter().map(|r| &r[..5]).collect::<Vec<_>>(), vec!["b.com", "c.com", "a.com", "d.com"]);
    }
}
