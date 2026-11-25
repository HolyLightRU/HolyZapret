use crate::desync::Config;
use crate::flows::{Action, Engine, Lists, Mode, QuicCfg, QuicMode, Stats};
use crate::lists::{Hosts, Nets};
use crate::procs::{self, Apps, ProcMap};
use crate::profiles::{self, Own, Ports};
use crate::sys;
use crate::svc;
use crate::windivert::{self, Address, Handle, Shutter};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const PATTERN: &str = "tls_clienthello_www_google_com.bin";
pub const WINWS_MUTEX: &str = "Global\\winws_windivert_mutex";
pub const PRIORITY: i16 = -100;

#[derive(Debug, PartialEq, Eq)]
pub struct Args {
    pub bin: PathBuf,
    pub lists: PathBuf,
    pub priority: i16,
    pub log: Option<PathBuf>,
    pub dry_run: bool,
    pub app_all: Vec<String>,
    pub app_skip: Vec<String>,
    pub verbose: bool,
    pub follow: Option<String>,
    pub service: bool,
    pub live: Option<PathBuf>,
    pub live_ms: u64,
    pub quic: Option<String>,
    pub autolist: Option<PathBuf>,
    pub own: Own,
    pub game_tcp: Ports,
    pub game_udp: Ports,
    pub experiment: String,
    pub memory: Option<PathBuf>,
    pub forward_watch: bool,
    pub forward: bool,
}

pub fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Args, String> {
    let mut bin = None;
    let mut lists = None;
    let mut priority = PRIORITY;
    let mut log = None;
    let mut dry_run = false;
    let mut app_all = Vec::new();
    let mut app_skip = Vec::new();
    let mut verbose = false;
    let mut follow = None;
    let mut service = false;
    let mut live = None;
    let mut live_ms = 2000u64;
    let mut quic = None;
    let mut autolist = None;
    let mut own = Own::default();
    let mut game_tcp = Ports::default();
    let mut game_udp = Ports::default();
    let mut experiment = "general".to_string();
    let mut memory = None;
    let mut forward_watch = false;
    let mut forward = false;
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        let (k, inline) = match a.split_once('=') {
            Some((k, v)) => (k.to_string(), Some(v.to_string())),
            None => (a.clone(), None),
        };
        let mut value = || inline.clone().or_else(|| it.next()).ok_or_else(|| format!("у {k} нет значения"));
        match k.as_str() {
            "--bin" => bin = Some(PathBuf::from(value()?)),
            "--lists" => lists = Some(PathBuf::from(value()?)),
            "--log" => log = Some(PathBuf::from(value()?)),
            "--priority" => {
                priority = value()?.parse().map_err(|_| "приоритет должен быть числом от -30000 до 30000".to_string())?;
                if !(-30000..=30000).contains(&priority) {
                    return Err("приоритет должен быть числом от -30000 до 30000".into());
                }
            }
            "--dry-run" => dry_run = true,
            "--forward-watch" => forward_watch = true,
            "--forward" => forward = true,
            "--app-all" => app_all.push(value()?),
            "--app-skip" => app_skip.push(value()?),
            "--verbose" => verbose = true,
            "--follow" => follow = Some(value()?),
            "--service" => service = true,
            "--live" => live = Some(PathBuf::from(value()?)),
            "--live-ms" => {
                live_ms = value()?.parse().map_err(|_| "--live-ms должен быть числом миллисекунд".to_string())?;
                live_ms = live_ms.clamp(100, 60000);
            }
            "--memory" => memory = Some(PathBuf::from(value()?)),
            "--own" => own = Own::parse(&value()?)?,
            "--autolist" => autolist = Some(PathBuf::from(value()?)),
            "--game-tcp" => game_tcp = Ports::parse(&value()?)?,
            "--game-udp" => game_udp = Ports::parse(&value()?)?,
            "--quic" => {
                let q = value()?.to_lowercase();
                if QuicMode::parse(&q).is_none() {
                    return Err(format!("неизвестный режим QUIC {q}: {}", QuicMode::NAMES.join(", ")));
                }
                quic = Some(q);
            }
            "--experiment" => {
                experiment = value()?.to_lowercase();
                if Mode::parse(&experiment).is_none() {
                    let names: Vec<&str> = crate::desync::TECHNIQUES.iter().map(|t| t.name).collect();
                    return Err(format!("неизвестный эксперимент {experiment}: {}, adaptive", names.join(", ")));
                }
            }
            other => return Err(format!("неизвестный параметр {other}")),
        }
    }
    Ok(Args { bin: bin.ok_or("не указан --bin")?, lists: lists.ok_or("не указан --lists")?, priority, log, dry_run, app_all, app_skip, verbose, follow, service, live, live_ms, quic, autolist, own, game_tcp, game_udp, experiment, memory, forward_watch, forward })
}

pub fn load_lists(dir: &Path) -> Lists {
    let f = |n: &str| dir.join(n);
    Lists {
        hosts: Hosts::load(&[&f("list-general.txt"), &f("list-general-user.txt")]),
        host_excludes: Hosts::load(&[&f("list-exclude.txt"), &f("list-exclude-user.txt")]),
        ip_excludes: Nets::load(&[&f("ipset-exclude.txt"), &f("ipset-exclude-user.txt")]),
        google: Hosts::load(&[&f("list-google.txt")]),
        ipset: Nets::load(&[&f("ipset-all-user.txt")]),
        ..Default::default()
    }
}

pub const FAKE_HTTP: &str = "tls_clienthello_www_sferum_ru.bin";
pub const FAKE_VOICE: &str = "ACTIVE_DISCORD_UDP.bin";
pub const FAKE_GAME_UDP: &str = "ACTIVE_GAME_UDP.bin";

fn port_clause(field: &str, ranges: &[(u16, u16)]) -> Vec<String> {
    ranges.iter().map(|(a, b)| if a == b { format!("{field} == {a}") } else { format!("({field} >= {a} and {field} <= {b})") }).collect()
}

fn filter_what(quic: bool, own: &Own, game_tcp: &Ports, game_udp: &Ports) -> String {
    let mut tcp_ports = vec!["tcp.DstPort == 443".to_string()];
    if own.http || own.myip_tcp {
        tcp_ports.push("tcp.DstPort == 80".into());
    }
    let mut extra: Vec<u16> = Vec::new();
    if own.cf {
        extra.extend(profiles::CF_PORTS);
    }
    if own.myip_tcp && !extra.contains(&8443) {
        extra.push(8443);
    }
    tcp_ports.extend(extra.iter().map(|p| format!("tcp.DstPort == {p}")));
    if own.game_tcp {
        tcp_ports.extend(port_clause("tcp.DstPort", game_tcp.ranges()));
    }
    let flags = "(tcp.Syn or tcp.Rst or tcp.Fin or tcp.PayloadLength > 0)";
    let tcp = if tcp_ports.len() == 1 { format!("tcp and {} and {flags}", tcp_ports[0]) } else { format!("tcp and ({}) and {flags}", tcp_ports.join(" or ")) };
    let mut udp: Vec<String> = Vec::new();
    if quic {
        udp.push("(udp and udp.DstPort == 443 and udp.PayloadLength >= 1200 and udp.Payload[0] >= 0xC0 and udp.Payload[0] <= 0xCF)".into());
    }
    if own.voice {
        let ports = port_clause("udp.DstPort", &profiles::VOICE).join(" or ");
        udp.push(format!("(udp and ({ports}) and ((udp.PayloadLength == 74 and udp.Payload16[0] == 0x0001) or (udp.PayloadLength >= 20 and udp.Payload32[1] == 0x2112A442)))"));
    }
    if own.game_udp && !game_udp.is_empty() {
        udp.push(format!("(udp and ({}))", port_clause("udp.DstPort", game_udp.ranges()).join(" or ")));
    }
    if udp.is_empty() { tcp } else { format!("(({tcp}) or {})", udp.join(" or ")) }
}

pub fn filter_for(quic: bool, own: &Own, game_tcp: &Ports, game_udp: &Ports) -> String {
    local_filter(&filter_what(quic, own, game_tcp, game_udp))
}

pub fn forward_filter(quic: bool, own: &Own, game_tcp: &Ports, game_udp: &Ports) -> String {
    let (v4, v6) = local_parts();
    format!("!impostor and {} and ((ip and {v4}) or (ipv6 and {v6}))", filter_what(quic, own, game_tcp, game_udp))
}

pub fn forward_lists(args: &Args) -> Lists {
    let mut lists = load_lists(&args.lists);
    lists.own = args.own;
    if let Some(p) = &args.autolist {
        lists.auto = Hosts::load(&[p]);
    }
    lists.game_tcp = args.game_tcp.clone();
    lists.game_udp = args.game_udp.clone();
    lists
}

pub fn filter(quic: bool) -> String {
    filter_for(quic, &Own::default(), &Ports::default(), &Ports::default())
}

fn local_filter(what: &str) -> String {
    let (v4, v6) = local_parts();
    format!("outbound and !impostor and !loopback and {what} and ((ip and {v4}) or (ipv6 and {v6}))")
}

pub const QUIC_FAKE: &str = "quic_initial_www_google_com.bin";

fn local_parts() -> (String, String) {
    let v4 = [
        ("0.0.0.0", "0.255.255.255"),
        ("10.0.0.0", "10.255.255.255"),
        ("100.64.0.0", "100.127.255.255"),
        ("127.0.0.0", "127.255.255.255"),
        ("169.254.0.0", "169.254.255.255"),
        ("172.16.0.0", "172.31.255.255"),
        ("192.168.0.0", "192.168.255.255"),
        ("224.0.0.0", "255.255.255.255"),
    ]
    .iter()
    .map(|(a, b)| format!("(ip.DstAddr < {a} or ip.DstAddr > {b})"))
    .collect::<Vec<_>>()
    .join(" and ");
    let v6 = [("fc00::", "fdff:ffff:ffff:ffff:ffff:ffff:ffff:ffff"), ("fe80::", "febf:ffff:ffff:ffff:ffff:ffff:ffff:ffff"), ("ff00::", "ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff")]
        .iter()
        .map(|(a, b)| format!("(ipv6.DstAddr < {a} or ipv6.DstAddr > {b})"))
        .collect::<Vec<_>>()
        .join(" and ");
    let v6 = format!("ipv6.DstAddr > ::1 and {v6}");
    (v4, v6)
}

pub struct Log {
    file: Option<Mutex<std::fs::File>>,
}

impl Log {
    pub fn open(path: Option<&Path>) -> Log {
        let big = path.and_then(|p| std::fs::metadata(p).ok()).is_some_and(|m| m.len() > 1024 * 1024);
        let file = path
            .and_then(|p| if big { std::fs::File::create(p).ok() } else { std::fs::OpenOptions::new().create(true).append(true).open(p).ok() })
            .map(Mutex::new);
        Log { file }
    }

    pub fn line(&self, text: &str) {
        let l = format!("{} {text}\n", sys::clock());
        let _ = std::io::stdout().write_all(l.as_bytes());
        if let Some(f) = &self.file {
            if let Ok(mut f) = f.lock() {
                let _ = f.write_all(l.as_bytes());
            }
        }
    }
}

pub fn stats_line(s: &Stats, flows: usize) -> String {
    let mut l = format!(
        "обработано {}, пропущено {}, без TCP timestamps {}, запасной приём {}, отпущено без изменений {}, потоков {}",
        s.handled, s.skipped, s.no_ts, s.fallback, s.flushed, flows
    );
    if s.answered + s.reset + s.silent > 0 {
        l.push_str(&format!("; итоги: ответ {}, сброс {}, тишина {}", s.answered, s.reset, s.silent));
        for (name, t) in &s.by_tech {
            l.push_str(&format!("; {name}: {} из {}", t.answered, t.answered + t.reset + t.silent));
        }
    }
    if s.voice + s.game_udp > 0 {
        l.push_str(&format!(", голос Discord {}, игры UDP {}", s.voice, s.game_udp));
    }
    if s.app_all + s.app_skipped > 0 {
        l.push_str(&format!(", целиком по программе {}, программ в исключениях {}", s.app_all, s.app_skipped));
    }
    l
}

fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o
}

fn opt(v: &Option<String>) -> String {
    v.as_deref().map(|s| format!("\"{}\"", esc(s))).unwrap_or_else(|| "null".into())
}

pub fn live_json(s: &Stats, flows: usize, recent: &std::collections::VecDeque<crate::flows::Decision>, running: bool, mode: &str, gateway: bool) -> String {
    let items: Vec<String> = recent
        .iter()
        .map(|d| {
            format!(
                "{{\"time\":\"{}\",\"host\":{},\"app\":{},\"pid\":{},\"handled\":{},\"why\":\"{}\",\"technique\":{},\"outcome\":{},\"parts\":{}}}",
                esc(&d.time),
                opt(&d.host),
                opt(&d.app),
                d.pid.map(|p| p.to_string()).unwrap_or_else(|| "null".into()),
                d.handled,
                esc(d.why),
                opt(&d.technique.map(String::from)),
                opt(&d.outcome.map(|o| o.name().to_string())),
                d.parts
            )
        })
        .collect();
    let techs: Vec<String> = s
        .by_tech
        .iter()
        .map(|(n, t)| format!("{{\"name\":\"{}\",\"used\":{},\"answered\":{},\"reset\":{},\"silent\":{}}}", esc(n), t.used, t.answered, t.reset, t.silent))
        .collect();
    format!(
        "{{\"running\":{running},\"gateway\":{gateway},\"time\":\"{}\",\"mode\":\"{}\",\"handled\":{},\"skipped\":{},\"no_ts\":{},\"fallback\":{},\"voice\":{},\"game_udp\":{},\"auto_added\":{},\"flushed\":{},\"flows\":{flows},\"answered\":{},\"reset\":{},\"silent\":{},\"techniques\":[{}],\"recent\":[{}]}}",
        esc(&sys::clock()),
        esc(mode),
        s.handled,
        s.skipped,
        s.no_ts,
        s.fallback,
        s.voice,
        s.game_udp,
        s.auto_added,
        s.flushed,
        s.answered,
        s.reset,
        s.silent,
        techs.join(","),
        items.join(",")
    )
}

pub fn inbound_filter(quic: bool) -> String {
    let tcp = "tcp and tcp.SrcPort == 443 and ((tcp.Syn and tcp.Ack) or tcp.Rst or (tcp.PayloadLength > 5 and tcp.Payload[0] == 0x16 and tcp.Payload[5] == 0x02))";
    if quic {
        format!("inbound and !loopback and (({tcp}) or (udp and udp.SrcPort == 443 and udp.PayloadLength > 6 and udp.Payload[0] >= 0xC0))")
    } else {
        format!("inbound and !loopback and {tcp}")
    }
}

fn append_lines(path: &Path, lines: &[String]) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        for l in lines {
            let _ = f.write_all(format!("{l}\r\n").as_bytes());
        }
    }
}

fn write_live(path: &Path, text: &str) {
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, text).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

fn names(set: &std::collections::HashSet<String>) -> String {
    let mut v: Vec<&str> = set.iter().map(String::as_str).collect();
    v.sort();
    v.join(", ")
}

pub(crate) fn deliver(h: &Handle, log: &Log, out: Vec<(Vec<u8>, Address)>) {
    for (p, a) in out {
        if !h.send_built(&p, &a) {
            log.line("не удалось отправить пакет");
        }
    }
}

static CURRENT: Mutex<Vec<Shutter>> = Mutex::new(Vec::new());

fn shut_current() {
    if let Ok(v) = CURRENT.lock() {
        for s in v.iter() {
            s.shutdown();
        }
    }
}

fn prepare_lists(args: &Args, log: &Log) -> Result<Lists, String> {
    let mut lists = load_lists(&args.lists);
    lists.apps = Apps::parse(&args.app_all, &args.app_skip);
    lists.own = args.own;
    if let Some(p) = &args.autolist {
        lists.auto = Hosts::load(&[p]);
        log.line(&format!("автосписок: сайтов {}", lists.auto.len()));
    }
    lists.game_tcp = args.game_tcp.clone();
    lists.game_udp = args.game_udp.clone();
    let owned = args.own.names();
    if !owned.is_empty() {
        log.line(&format!("профили general: {}, «Мои IP» адресов {}, google доменов {}", owned.join(", "), lists.ipset.as_slice().len(), lists.google.len()));
    }
    if lists.hosts.is_empty() {
        return Err(format!("в {} нет list-general.txt или он пустой", args.lists.display()));
    }
    log.line(&format!(
        "списки: доменов {}, исключённых доменов {}, исключённых сетей {}",
        lists.hosts.len(),
        lists.host_excludes.len(),
        lists.ip_excludes.as_slice().len()
    ));
    if !lists.apps.all.is_empty() {
        log.line(&format!("обходить целиком: {}", names(&lists.apps.all)));
    }
    if !lists.apps.skip.is_empty() {
        log.line(&format!("не трогать: {}", names(&lists.apps.skip)));
    }
    Ok(lists)
}

pub fn run(args: Args) -> Result<(), String> {
    let log = Arc::new(Log::open(args.log.as_deref()));
    log.line(&format!("RayOfLight {} запускается", env!("CARGO_PKG_VERSION")));
    let hook_log = log.clone();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current().name().unwrap_or("без имени").to_string();
        hook_log.line(&format!("сбой в потоке {thread}: {info}"));
    }));
    let r = run_logged(&args, &log);
    if let Err(e) = &r {
        log.line(&format!("ошибка: {e}"));
    }
    r
}

fn run_logged(args: &Args, log: &Arc<Log>) -> Result<(), String> {
    let pattern = std::fs::read(args.bin.join(PATTERN)).map_err(|e| format!("не прочитан {}: {e}", args.bin.join(PATTERN).display()))?;
    let mut cfg = Config::general(pattern).ok_or("шаблон seqovl пустой или встроенный фейк испорчен")?;
    let read = |n: &str| std::fs::read(args.bin.join(n)).map_err(|e| format!("не прочитан {}: {e}", args.bin.join(n).display()));
    if args.own.http || args.own.myip_tcp || args.own.game_tcp {
        cfg.fake_http = read(FAKE_HTTP)?;
    }
    if args.own.voice {
        cfg.fake_voice = read(FAKE_VOICE)?;
    }
    if args.own.game_udp {
        cfg.fake_game_udp = read(FAKE_GAME_UDP)?;
    }
    let first = prepare_lists(args, log)?;
    log.line(&format!("эксперимент: {}", args.experiment));
    let quic = match args.quic.as_deref().and_then(QuicMode::parse) {
        Some(mode) => {
            let path = args.bin.join(QUIC_FAKE);
            let fake = std::fs::read(&path).map_err(|e| format!("не прочитан {}: {e}", path.display()))?;
            let q = QuicCfg::new(mode, fake).ok_or_else(|| format!("{} не похож на первый пакет QUIC", path.display()))?;
            log.line(&format!("QUIC: {}", mode.tech()));
            Some(q)
        }
        None => None,
    };
    let lib = windivert::Lib::load(&args.bin)?;
    if args.forward_watch {
        if args.dry_run {
            lib.check(crate::watch::FORWARD_FILTER, windivert::LAYER_NETWORK_FORWARD)?;
            lib.check(crate::watch::REDIRECT_FILTER, windivert::LAYER_NETWORK)?;
            log.line("проверка без запуска: фильтры наблюдения в порядке");
            return Ok(());
        }
        return crate::watch::run(&lib, args.priority, log);
    }
    if args.dry_run {
        lib.check(&filter_for(args.quic.is_some(), &args.own, &args.game_tcp, &args.game_udp), windivert::LAYER_NETWORK)?;
        lib.check(&inbound_filter(args.quic.is_some()), windivert::LAYER_NETWORK)?;
        if args.forward {
            lib.check(&forward_filter(args.quic.is_some(), &args.own, &args.game_tcp, &args.game_udp), windivert::LAYER_NETWORK_FORWARD)?;
            lib.check(crate::watch::REDIRECT_FILTER, windivert::LAYER_NETWORK)?;
            log.line("шлюз: фильтры в порядке");
        }
        log.line("проверка без запуска: шаблон, списки, фильтры и WinDivert.dll в порядке");
        return Ok(());
    }
    sys::on_stop(shut_current);
    let Some(name) = args.follow.as_deref() else {
        return session(args, &lib, cfg, quic, first, log, None);
    };
    log.line(&format!("слежу за службой {name}: перехват работает, только пока она запущена"));
    let mut lists = Some(first);
    while !sys::stopping() {
        if !svc::running(name) {
            std::thread::sleep(Duration::from_secs(1));
            continue;
        }
        let l = match lists.take() {
            Some(l) => l,
            None => match prepare_lists(args, log) {
                Ok(l) => l,
                Err(e) => {
                    log.line(&format!("списки не прочитаны: {e}"));
                    std::thread::sleep(Duration::from_secs(5));
                    continue;
                }
            },
        };
        if let Err(e) = session(args, &lib, cfg.clone(), quic.clone(), l, log, Some(name)) {
            log.line(&format!("перехват не включился: {e}"));
            std::thread::sleep(Duration::from_secs(5));
        }
    }
    log.line("RayOfLight остановлен");
    Ok(())
}

fn session(args: &Args, lib: &windivert::Lib, cfg: Config, quic: Option<QuicCfg>, lists: Lists, log: &Arc<Log>, follow: Option<&str>) -> Result<(), String> {
    let track_apps = !lists.apps.is_empty() || args.verbose || args.live.is_some();
    let handle = {
        let _lock = sys::GlobalLock::take(WINWS_MUTEX);
        lib.open(&filter_for(args.quic.is_some(), &args.own, &args.game_tcp, &args.game_udp), windivert::LAYER_NETWORK, args.priority, 0)?
    };
    let h = Arc::new(handle);
    let procmap = Arc::new(Mutex::new(None::<ProcMap>));
    let sockets = if track_apps {
        match lib.open("true", windivert::LAYER_SOCKET, 0, windivert::FLAG_SNIFF | windivert::FLAG_RECV_ONLY) {
            Ok(s) => {
                *procmap.lock().unwrap() = Some(ProcMap::system());
                Some(Arc::new(s))
            }
            Err(e) => {
                log.line(&format!("программы не определяются: {e}"));
                None
            }
        }
    } else {
        None
    };
    let inbound = match lib.open(&inbound_filter(args.quic.is_some()), windivert::LAYER_NETWORK, args.priority, windivert::FLAG_SNIFF | windivert::FLAG_RECV_ONLY) {
        Ok(i) => Some(Arc::new(i)),
        Err(e) => {
            log.line(&format!("итоги соединений не отслеживаются: {e}"));
            None
        }
    };
    if let Ok(mut v) = CURRENT.lock() {
        v.clear();
        v.push(h.shutter());
        if let Some(s) = &sockets {
            v.push(s.shutter());
        }
        if let Some(i) = &inbound {
            v.push(i.shutter());
        }
    }
    if sys::stopping() {
        shut_current();
    }
    let ended = Arc::new(AtomicBool::new(false));
    let watcher = follow.map(|name| {
        let (ended, log, name) = (ended.clone(), log.clone(), name.to_string());
        std::thread::spawn(move || {
            while !ended.load(Ordering::SeqCst) && !sys::stopping() {
                std::thread::sleep(Duration::from_millis(500));
                if !svc::running(&name) {
                    log.line(&format!("служба {name} остановлена, отпускаю перехват"));
                    shut_current();
                    break;
                }
            }
        })
    });
    let socket_thread = sockets.clone().map(|s| {
        let procmap = procmap.clone();
        std::thread::spawn(move || {
            while let Some(a) = s.recv_event() {
                if let Some(ev) = procs::decode_socket(&a) {
                    if let Ok(mut m) = procmap.lock() {
                        if let Some(m) = m.as_mut() {
                            m.on_event(&ev);
                        }
                    }
                }
            }
        })
    });
    log.line(&format!("перехват включён, приоритет {}", args.priority));
    let gateway = if args.forward {
        match crate::gateway::start(args, lib, cfg.clone(), quic.clone(), log) {
            Ok(g) => {
                if let Ok(mut v) = CURRENT.lock() {
                    v.extend(g.shutters());
                }
                if sys::stopping() {
                    shut_current();
                }
                Some(Arc::new(g))
            }
            Err(e) => {
                log.line(&format!("шлюз не включился: {e}"));
                None
            }
        }
    } else {
        None
    };
    let mode = Mode::parse(&args.experiment).unwrap_or(Mode::Fixed(crate::desync::GENERAL));
    let mut eng = Engine::new(cfg, lists, Box::new(sys::random)).with_clock(sys::clock).with_mode(mode);
    if let Some(q) = quic {
        eng = eng.with_quic(q);
    }
    eng.autolist = args.autolist.is_some();
    eng.verbose = args.verbose;
    eng.track = args.live.is_some();
    if let Some(p) = args.memory.as_deref() {
        let n = eng.load_memory(&std::fs::read_to_string(p).unwrap_or_default());
        if n > 0 {
            log.line(&format!("самоподстройка: память по {n} сайтам загружена"));
        }
    }
    if sockets.is_some() {
        let procmap = procmap.clone();
        eng = eng.with_apps(Box::new(move |c, now| procmap.lock().ok()?.as_mut()?.owner(c, now)));
    }
    let engine: Arc<Mutex<Engine<Address>>> = Arc::new(Mutex::new(eng));
    let inbound_thread = inbound.clone().map(|i| {
        let engine = engine.clone();
        std::thread::spawn(move || {
            let mut buf = vec![0u8; windivert::MAX_PACKET];
            while let Some((len, _)) = i.recv(&mut buf) {
                if let Ok(mut e) = engine.lock() {
                    e.observe(&buf[..len], Instant::now());
                }
            }
        })
    });
    let ticker = {
        let gw = gateway.clone();
        let (h, engine, log, procmap, ended, live, memory, live_every, autolist) = (
            h.clone(),
            engine.clone(),
            log.clone(),
            procmap.clone(),
            ended.clone(),
            args.live.clone(),
            args.memory.clone(),
            Duration::from_millis(args.live_ms),
            args.autolist.clone(),
        );
        std::thread::spawn(move || {
            let mut act = crate::activity::Activity::new(Instant::now());
            let mut last = Instant::now();
            let mut saved = Instant::now();
            let mut written = Instant::now() - Duration::from_secs(10);
            let mut cleaned = Instant::now();
            while !ended.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(100));
                let (out, notes, added) = engine.lock().map(|mut e| (e.expire(Instant::now()), std::mem::take(&mut e.notes), e.take_auto_new())).unwrap_or_default();
                if let Some(p) = autolist.as_deref().filter(|_| !added.is_empty()) {
                    append_lines(p, &added);
                    log.line(&format!("в автосписок добавлены: {}", added.join(", ")));
                }
                deliver(&h, &log, out);
                for n in notes {
                    log.line(&n);
                }
                if let Some(g) = &gw {
                    g.tick(&log);
                }
                if let Some(p) = live.as_deref().filter(|_| written.elapsed() >= live_every) {
                    written = Instant::now();
                    let extra = gw.as_ref().map(|g| g.recent());
                    let side = gw.as_ref().map(|g| g.totals()).unwrap_or_default();
                    if let Ok(e) = engine.lock() {
                        let s = &e.stats;
                        let now = Instant::now();
                        act.update((s.handled + side.0, s.answered + side.1, s.reset + s.silent + side.2), now);
                        act.load(sys::cpu_time(), sys::memory(), now);
                        let json = live_json(&e.stats, e.flows(), &crate::gateway::merge(&e.recent, extra), true, e.mode().name(), gw.is_some());
                        write_live(p, &crate::activity::with_extra(&json, &act.json()));
                    }
                }
                if let Some(p) = memory.as_deref().filter(|_| saved.elapsed() >= Duration::from_secs(30)) {
                    saved = Instant::now();
                    if let Ok(e) = engine.lock() {
                        let text = e.memory_text();
                        if !text.is_empty() {
                            write_live(p, &text);
                        }
                    }
                }
                if cleaned.elapsed() >= Duration::from_secs(60) {
                    cleaned = Instant::now();
                    let alive: std::collections::HashSet<u32> = procs::process_names().into_keys().collect();
                    if let Ok(mut m) = procmap.lock() {
                        if let Some(m) = m.as_mut() {
                            m.forget_dead(&alive);
                        }
                    }
                }
                if last.elapsed() >= Duration::from_secs(30) {
                    last = Instant::now();
                    if let Ok(e) = engine.lock() {
                        log.line(&format!("{}, ждут итога {}", stats_line(&e.stats, e.flows()), e.pending()));
                    }
                    if let Some(g) = &gw {
                        log.line(&g.stats_line());
                    }
                }
            }
        })
    };
    let mut buf = vec![0u8; windivert::MAX_PACKET];
    while let Some((len, addr)) = h.recv(&mut buf) {
        let raw = &buf[..len];
        let act = engine.lock().map(|mut e| e.handle(raw, addr, Instant::now())).unwrap_or(Action::Pass);
        match act {
            Action::Pass => {
                if !h.send(raw, &addr) {
                    log.line("не удалось вернуть пакет");
                }
            }
            Action::Hold | Action::Drop => {}
            Action::Send(out) => deliver(&h, log, out),
        }
    }
    ended.store(true, Ordering::SeqCst);
    shut_current();
    let _ = ticker.join();
    if let Some(t) = socket_thread {
        let _ = t.join();
    }
    if let Some(t) = watcher {
        let _ = t.join();
    }
    if let Some(t) = inbound_thread {
        let _ = t.join();
    }
    let rest = engine.lock().map(|mut e| e.drain()).unwrap_or_default();
    deliver(&h, log, rest);
    if let Some(g) = &gateway {
        g.finish(log);
    }
    let extra = gateway.as_ref().map(|g| g.recent());
    let notes = engine.lock().map(|mut e| std::mem::take(&mut e.notes)).unwrap_or_default();
    for n in notes {
        log.line(&n);
    }
    if let Some(m) = procmap.lock().ok().as_ref().and_then(|m| m.as_ref()) {
        log.line(&format!("событий соединения от System (pid 4) пропущено: {}, соединений в карте: {}", m.system_events, m.tracked()));
    }
    if let Ok(mut e) = engine.lock() {
        let _ = e.expire(Instant::now());
        for n in std::mem::take(&mut e.notes) {
            log.line(&n);
        }
        log.line(&format!("{}, ждут итога {}", stats_line(&e.stats, e.flows()), e.pending()));
        if let Some(p) = args.live.as_deref() {
            write_live(p, &live_json(&e.stats, e.flows(), &crate::gateway::merge(&e.recent, extra), false, e.mode().name(), gateway.is_some()));
        }
        if let Some(p) = args.memory.as_deref() {
            let text = e.memory_text();
            if !text.is_empty() {
                write_live(p, &text);
            }
        }
    }
    if let Ok(mut v) = CURRENT.lock() {
        v.clear();
    }
    log.line("перехват выключен");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn parses_arguments() {
        let a = parse_args(s(&["--bin", "C:\\hz\\bin", "--lists=C:\\hz\\exp-list", "--dry-run"])).unwrap();
        assert_eq!(
            a,
            Args { bin: "C:\\hz\\bin".into(), lists: "C:\\hz\\exp-list".into(), priority: -100, log: None, dry_run: true, app_all: vec![], app_skip: vec![], verbose: false, follow: None, service: false, live: None, live_ms: 2000, quic: None, autolist: None, own: Own::default(), game_tcp: Ports::default(), game_udp: Ports::default(), experiment: "general".into(), memory: None, forward_watch: false, forward: false }
        );
        let b = parse_args(s(&["--bin", "b", "--lists", "l", "--app-all", "discord,chrome.exe", "--app-skip=steam", "--app-all", "curl", "--verbose"])).unwrap();
        assert_eq!((b.app_all, b.app_skip, b.verbose), (vec!["discord,chrome.exe".to_string(), "curl".to_string()], vec!["steam".to_string()], true));
        let c = parse_args(s(&["--bin", "b", "--lists", "l", "--service", "--follow", "zapret", "--live", "x.json"])).unwrap();
        assert_eq!((c.service, c.follow.as_deref(), c.live, c.live_ms), (true, Some("zapret"), Some(PathBuf::from("x.json")), 2000));
        assert_eq!(parse_args(s(&["--bin", "b", "--lists", "l", "--live-ms", "300"])).unwrap().live_ms, 300);
        assert_eq!(parse_args(s(&["--bin", "b", "--lists", "l", "--live-ms", "5"])).unwrap().live_ms, 100);
        assert!(parse_args(s(&["--bin", "b", "--lists", "l", "--experiment", "nope"])).unwrap_err().contains("ttl"));
        let p = parse_args(s(&["--bin", "b", "--lists", "l", "--own", "http,voice", "--game-tcp", "1024-65535", "--game-udp", "12"])).unwrap();
        assert!(p.own.http && p.own.voice && p.game_tcp.contains(5000) && p.game_udp.contains(12));
        assert!(parse_args(s(&["--bin", "b", "--lists", "l", "--own", "x"])).is_err());
        let e = parse_args(s(&["--bin", "b", "--lists", "l", "--experiment", "Adaptive", "--memory", "m.tsv"])).unwrap();
        assert_eq!((e.experiment.as_str(), e.memory), ("adaptive", Some(PathBuf::from("m.tsv"))));
        assert!(parse_args(s(&["--bin", "b", "--lists", "l", "--experiment", "magic"])).is_err());
        assert_eq!(parse_args(s(&["--bin", "b", "--lists", "l", "--priority", "5", "--log", "x.log"])).unwrap().priority, 5);
        assert!(parse_args(s(&["--bin", "b"])).is_err());
        assert!(parse_args(s(&["--bin", "b", "--lists", "l", "--priority", "99999"])).is_err());
        assert!(parse_args(s(&["--bin", "b", "--lists", "l", "--what"])).is_err());
        assert!(parse_args(s(&["--bin"])).is_err());
    }

    #[test]
    fn live_json_is_valid_shape() {
        let mut q = std::collections::VecDeque::new();
        q.push_back(crate::flows::Decision {
            id: 1,
            technique: Some("general"),
            outcome: Some(crate::flows::Outcome::Reset),
            time: "12:00:00".into(),
            host: Some("a\"b.com".into()),
            app: None,
            pid: Some(5),
            handled: true,
            why: "сайт в списке", parts: 2 });
        let mut s = Stats { handled: 3, reset: 1, ..Default::default() };
        s.by_tech.insert("general", crate::flows::TechStats { used: 3, answered: 2, reset: 1, silent: 0 });
        let j = live_json(&s, 2, &q, true, "adaptive", false);
        assert!(j.starts_with("{\"running\":true,"));
        assert!(j.contains("\"mode\":\"adaptive\"") && j.contains("\"handled\":3,") && j.contains("\"flows\":2,") && j.contains("\"reset\":1,"));
        assert!(j.contains("\"techniques\":[{\"name\":\"general\",\"used\":3,\"answered\":2,\"reset\":1,\"silent\":0}]"));
        assert!(j.contains("{\"time\":\"12:00:00\",\"host\":\"a\\\"b.com\",\"app\":null,\"pid\":5,\"handled\":true,\"why\":\"сайт в списке\",\"technique\":\"general\",\"outcome\":\"reset\",\"parts\":2}"));
        assert_eq!(j.matches('{').count(), j.matches('}').count());
    }

    #[test]
    fn filter_is_outbound_https_only() {
        let f = filter(false);
        assert!(f.starts_with("outbound and !impostor and !loopback and tcp and tcp.DstPort == 443"));
        let all = Own::parse("http,google,cf,myip-tcp,myip-quic,game-tcp,game-udp,voice").unwrap();
        let wide = filter_for(true, &all, &Ports::parse("1024-65535").unwrap(), &Ports::parse("1024-65535").unwrap());
        assert!(wide.contains("tcp.DstPort == 80 or tcp.DstPort == 2053") && wide.contains("udp.Payload32[1] == 0x2112A442") && wide.contains("(udp.DstPort >= 1024 and udp.DstPort <= 65535)"));
        assert_eq!(wide.matches('(').count(), wide.matches(')').count());
        assert_eq!(wide.matches("8443").count(), 1);
        let q = filter(true);
        assert!(q.contains(" or (udp and udp.DstPort == 443 and udp.PayloadLength >= 1200"));
        assert_eq!(q.matches('(').count(), q.matches(')').count());
        let i = inbound_filter(true);
        assert!(i.contains("udp.SrcPort == 443") && !inbound_filter(false).contains("udp"));
        assert_eq!(i.matches('(').count(), i.matches(')').count());
        assert!(f.contains("(ip.DstAddr < 192.168.0.0 or ip.DstAddr > 192.168.255.255)"));
        assert!(f.contains("(ipv6 and ipv6.DstAddr > ::1 and (ipv6.DstAddr < fc00:: or"));
        assert_eq!(f.matches('(').count(), f.matches(')').count());
    }

    #[test]
    fn loads_lists_from_folder() {
        let dir = std::env::temp_dir().join(format!("rayoflight-lists-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("list-general.txt"), "youtube.com\r\ndiscord.com\r\n").unwrap();
        std::fs::write(dir.join("list-general-user.txt"), "domain.example.abc\r\n").unwrap();
        std::fs::write(dir.join("list-exclude.txt"), "pass.youtube.com\r\n").unwrap();
        std::fs::write(dir.join("ipset-exclude-user.txt"), "203.0.113.113/32\r\n").unwrap();
        let l = load_lists(&dir);
        assert_eq!((l.hosts.len(), l.host_excludes.len(), l.ip_excludes.as_slice().len()), (3, 1, 1));
        assert!(l.hosts.matches("www.youtube.com") && l.host_excludes.matches("pass.youtube.com"));
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(load_lists(&dir).hosts.is_empty());
    }

    #[test]
    fn dry_run_checks_files() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let base = Args { bin: root.join("bin"), lists: root.join("exp-list"), priority: -100, log: None, dry_run: true, app_all: vec!["curl".into()], app_skip: vec![], verbose: false, follow: None, service: false, live: None, live_ms: 2000, quic: None, autolist: None, own: Own::default(), game_tcp: Ports::default(), game_udp: Ports::default(), experiment: "general".into(), memory: None, forward_watch: false, forward: false };
        assert_eq!(run(base), Ok(()));
        let a = Args { bin: root.join("bin"), lists: root.join("нет-такой-папки"), priority: -100, log: None, dry_run: true, app_all: vec![], app_skip: vec![], verbose: false, follow: None, service: false, live: None, live_ms: 2000, quic: None, autolist: None, own: Own::default(), game_tcp: Ports::default(), game_udp: Ports::default(), experiment: "general".into(), memory: None, forward_watch: false, forward: false };
        assert!(run(a).unwrap_err().contains("list-general.txt"));
    }
}
