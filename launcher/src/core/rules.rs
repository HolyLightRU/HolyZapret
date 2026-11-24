use super::gamefilter::GameFilter;
use super::paths::Paths;
use super::picker::{self, Check, CheckKind, Target};
use super::service::{self, ServiceState};
use super::strategy::{self, Strategy};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

pub const NONE: &str = "none";
pub const BORROW: &str = "from:";
const BORROW_OPTIONS: [&str; 4] = ["--dpi-desync", "--ip-id", "--dup", "--orig"];
const FILE: &str = "site-rules-user.txt";
const MAX_DOMAINS: usize = 16;

pub struct Technique {
    pub id: &'static str,
    pub name: &'static str,
    pub args: &'static str,
}

const PATTERN: &str = "--dpi-desync-split-seqovl-pattern=\"{BIN}tls_clienthello_www_google_com.bin\"";

pub const TECHNIQUES: &[Technique] = &[
    Technique {
        id: "alt3-ts",
        name: "ALT3, подмена ts",
        args: "--dpi-desync=fake,multisplit --dpi-desync-split-seqovl=681 --dpi-desync-split-pos=1 --dpi-desync-fooling=ts --dpi-desync-repeats=8 {PATTERN} --dpi-desync-fake-tls-mod=rnd,dupsid,sni=www.google.com",
    },
    Technique {
        id: "alt3-badseq",
        name: "ALT3, подмена badseq",
        args: "--dpi-desync=fake,multisplit --dpi-desync-split-seqovl=681 --dpi-desync-split-pos=1 --dpi-desync-fooling=badseq --dpi-desync-repeats=8 {PATTERN} --dpi-desync-fake-tls-mod=rnd,dupsid,sni=www.google.com",
    },
    Technique {
        id: "alt3-md5sig",
        name: "ALT3, подмена md5sig",
        args: "--dpi-desync=fake,multisplit --dpi-desync-split-seqovl=681 --dpi-desync-split-pos=1 --dpi-desync-fooling=md5sig --dpi-desync-repeats=8 {PATTERN} --dpi-desync-fake-tls-mod=rnd,dupsid,sni=www.google.com",
    },
    Technique {
        id: "alt3-ts-badseq",
        name: "ALT3, подмена ts и badseq",
        args: "--dpi-desync=fake,multisplit --dpi-desync-split-seqovl=681 --dpi-desync-split-pos=1 --dpi-desync-fooling=ts,badseq --dpi-desync-repeats=8 {PATTERN} --dpi-desync-fake-tls-mod=rnd,dupsid,sni=www.google.com",
    },
    Technique {
        id: "alt3-autottl",
        name: "ALT3, подмена по TTL",
        args: "--dpi-desync=fake,multisplit --dpi-desync-split-seqovl=681 --dpi-desync-split-pos=1 --dpi-desync-autottl=2 --dpi-desync-repeats=8 {PATTERN} --dpi-desync-fake-tls-mod=rnd,dupsid,sni=www.google.com",
    },
    Technique {
        id: "seqovl",
        name: "Только наложение seqovl",
        args: "--dpi-desync=multisplit --dpi-desync-split-seqovl=681 --dpi-desync-split-pos=1 {PATTERN}",
    },
    Technique {
        id: "seqovl-sniext",
        name: "Наложение у SNI",
        args: "--dpi-desync=multisplit --dpi-desync-split-seqovl=679 --dpi-desync-split-pos=2,sniext+1 {PATTERN}",
    },
    Technique {
        id: "fakedsplit",
        name: "fakedsplit, badseq",
        args: "--dpi-desync=fake,fakedsplit --dpi-desync-split-pos=1 --dpi-desync-fooling=badseq --dpi-desync-repeats=6 --dpi-desync-fake-tls-mod=rnd,dupsid,sni=www.google.com",
    },
    Technique {
        id: "multidisorder",
        name: "multidisorder у домена",
        args: "--dpi-desync=multidisorder --dpi-desync-split-pos=1,midsld",
    },
    Technique {
        id: "fake-md5sig",
        name: "Подмена и разрез у домена, md5sig",
        args: "--dpi-desync=fake,multisplit --dpi-desync-split-pos=1,midsld --dpi-desync-fooling=md5sig --dpi-desync-repeats=6 --dpi-desync-fake-tls-mod=rnd,dupsid,sni=www.google.com",
    },
];

pub fn technique(id: &str) -> Option<&'static Technique> {
    TECHNIQUES.iter().find(|t| t.id == id)
}

pub fn label(id: &str) -> String {
    if id == NONE {
        return "Не обходить".into();
    }
    if let Some(name) = id.strip_prefix(BORROW) {
        return format!("Как в «{name}»");
    }
    technique(id).map(|t| t.name.to_string()).unwrap_or_else(|| id.to_string())
}

pub fn technique_args(t: &Technique, bin: &Path) -> String {
    let bin = format!("{}\\", bin.display());
    t.args.replace("{PATTERN}", PATTERN).replace("{BIN}", &bin)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rule {
    pub technique: String,
    pub domains: Vec<String>,
    pub origin: String,
    pub tcp: String,
    pub udp: String,
}

impl Rule {
    pub fn borrowed(&self) -> bool {
        self.technique.starts_with(BORROW)
    }

    pub fn line(&self) -> String {
        let base = format!("{} | {} | {}", self.technique, self.domains.join(", "), self.origin);
        if self.borrowed() { format!("{base} | {} | {}", self.tcp, self.udp) } else { base }
    }
}

fn split_args(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in s.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                cur.push(c);
            }
            ' ' if !quoted => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

pub fn safe_args(s: &str) -> bool {
    split_args(s).iter().all(|t| {
        let (k, v) = t.split_once('=').unwrap_or((t.as_str(), ""));
        let key_ok = BORROW_OPTIONS.iter().any(|p| k.starts_with(p)) && k.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        let value_ok = match v.strip_prefix("\"{BIN}").and_then(|r| r.strip_suffix('"')) {
            Some(file) => !file.is_empty() && file.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')) && !file.contains(".."),
            None => v.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, ',' | '.' | ':' | '-' | '_' | '+' | '^' | '!' | '=' | '@' | '~' | '#')),
        };
        key_ok && value_ok
    })
}

fn stored(tokens: &[String], bin: &Path) -> String {
    let prefix = format!("{}\\", bin.display()).to_lowercase();
    tokens
        .iter()
        .map(|t| {
            let (k, v) = t.split_once('=').unwrap_or((t.as_str(), ""));
            if v.to_lowercase().starts_with(&prefix) {
                format!("{k}=\"{{BIN}}{}\"", &v[prefix.len()..])
            } else {
                t.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn borrow_rule(paths: &Paths, donor: &Strategy, domains: Vec<String>, origin: &str) -> anyhow::Result<Rule> {
    let host = domains.first().cloned().ok_or_else(|| anyhow::anyhow!("нет доменов"))?;
    let args = strategy::extract_args(donor, paths, &GameFilter::default())?;
    let ips: Vec<std::net::IpAddr> = std::net::ToSocketAddrs::to_socket_addrs(&(host.as_str(), 443)).map(|a| a.map(|s| s.ip()).collect()).unwrap_or_default();
    let [tcp, udp] = super::coverage::borrow(&args, &host, &ips, &mut super::coverage::Cache::default());
    let tcp = tcp.map(|t| stored(&t, &donor.bin)).unwrap_or_default();
    let udp = udp.map(|t| stored(&t, &donor.bin)).unwrap_or_default();
    if tcp.is_empty() && udp.is_empty() {
        anyhow::bail!("стратегия «{}» не обходит {host}", donor.name);
    }
    if !safe_args(&tcp) || !safe_args(&udp) {
        anyhow::bail!("настройки стратегии «{}» нельзя перенести в правило", donor.name);
    }
    Ok(Rule { technique: format!("{BORROW}{}", donor.name), domains, origin: origin.to_string(), tcp, udp })
}

pub fn path(paths: &Paths) -> PathBuf {
    paths.root.join("utils").join(FILE)
}

pub fn parse(text: &str) -> Vec<Rule> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let mut parts = l.splitn(5, '|').map(str::trim);
            let technique = parts.next()?.to_string();
            let domains: Vec<String> = parts
                .next()?
                .split([',', ' '])
                .filter_map(|d| super::lists::normalize_domain(d).ok())
                .collect();
            let origin = parts.next().unwrap_or("").to_string();
            let borrowed = technique.starts_with(BORROW) && technique.len() > BORROW.len();
            let (tcp, udp) = if borrowed {
                (parts.next().unwrap_or("").to_string(), parts.next().unwrap_or("").to_string())
            } else {
                (String::new(), String::new())
            };
            let known = technique == NONE
                || self::technique(&technique).is_some()
                || (borrowed && (!tcp.is_empty() || !udp.is_empty()) && safe_args(&tcp) && safe_args(&udp));
            (known && !domains.is_empty()).then_some(Rule { technique, domains, origin, tcp, udp })
        })
        .collect()
}

pub fn render(rules: &[Rule]) -> String {
    let mut out = String::from("# Правила для сайтов HolyZapret: приём | домены | для какой стратегии подобрано\r\n");
    for r in rules {
        out.push_str(&r.line());
        out.push_str("\r\n");
    }
    out
}

pub fn load_or_empty(paths: Option<&Paths>) -> Vec<Rule> {
    paths.map(load).unwrap_or_default()
}

pub fn load(paths: &Paths) -> Vec<Rule> {
    parse(&fs::read_to_string(path(paths)).unwrap_or_default())
}

pub fn save(paths: &Paths, rules: &[Rule]) -> anyhow::Result<()> {
    let p = path(paths);
    if let Some(dir) = p.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = p.with_extension("txt.tmp");
    fs::write(&tmp, render(rules))?;
    fs::rename(&tmp, &p)?;
    Ok(())
}

pub fn upsert(rules: &mut Vec<Rule>, rule: Rule) {
    for r in rules.iter_mut() {
        r.domains.retain(|d| !rule.domains.iter().any(|n| n.eq_ignore_ascii_case(d)));
    }
    rules.retain(|r| !r.domains.is_empty());
    rules.push(rule);
}

pub fn profiles(rules: &[Rule], bin: &Path) -> String {
    let mut out = Vec::new();
    let dir = format!("{}\\", bin.display());
    for r in rules {
        let domains = r.domains.join(",");
        let body = if r.borrowed() {
            if r.tcp.is_empty() {
                continue;
            }
            format!(" {}", r.tcp.replace("{BIN}", &dir))
        } else {
            match technique(&r.technique) {
                Some(t) => format!(" {}", technique_args(t, bin)),
                None => String::new(),
            }
        };
        out.push(format!("--filter-tcp=80,443 --hostlist-domains={domains}{body}"));
    }
    for r in rules.iter().filter(|r| r.borrowed() && !r.udp.is_empty()) {
        out.push(format!("--filter-udp=443 --hostlist-domains={} {}", r.domains.join(","), r.udp.replace("{BIN}", &dir)));
    }
    let all: Vec<String> = rules.iter().flat_map(|r| r.domains.clone()).collect();
    if !all.is_empty() {
        out.push(format!("--filter-udp=443 --hostlist-domains={}", all.join(",")));
    }
    out.join(" --new ")
}

pub fn inject(args: &str, rules: &[Rule], bin: &Path) -> String {
    if rules.is_empty() {
        return args.to_string();
    }
    let at = args.match_indices("--filter-").map(|(i, _)| i).find(|&i| i == 0 || args.as_bytes()[i - 1] == b' ');
    let Some(i) = at else { return args.to_string() };
    format!("{}{} --new {}", &args[..i], profiles(rules, bin), &args[i..])
}

pub fn with_rules(paths: &Paths, s: &Strategy, gf: &GameFilter) -> anyhow::Result<String> {
    let args = strategy::extract_args(s, paths, gf)?;
    Ok(inject(&args, &load(paths), &paths.bin()))
}

#[derive(Clone, Debug)]
pub struct Trial {
    pub label: String,
    pub rule: Option<Rule>,
    pub ok: usize,
    pub total: usize,
    pub ms: u32,
}

impl Trial {
    fn from(label: impl Into<String>, checks: &[Check]) -> Trial {
        let ok: Vec<&Check> = checks.iter().filter(|c| c.ok).collect();
        let ms = if ok.is_empty() { 0 } else { ok.iter().map(|c| c.ms).sum::<u32>() / ok.len() as u32 };
        Trial { label: label.into(), rule: None, ok: ok.len(), total: checks.len(), ms }
    }

    pub fn passed(&self) -> bool {
        self.total > 0 && self.ok == self.total
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Advice {
    Fine,
    Works(String),
    Exclude,
    Use(Rule),
    Nothing,
}

#[derive(Clone, Debug)]
pub struct Verdict {
    pub url: String,
    pub domains: Vec<String>,
    pub baseline: Trial,
    pub strategy: Option<Trial>,
    pub techniques: Vec<Trial>,
    pub advice: Advice,
    pub origin: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleState {
    Works,
    NotNeeded,
    Broken,
}

pub fn rule_state(with_rule: &Trial, without_rule: &Trial) -> RuleState {
    match (with_rule.passed(), without_rule.passed()) {
        (true, true) => RuleState::NotNeeded,
        (true, false) => RuleState::Works,
        (false, _) => RuleState::Broken,
    }
}

#[derive(Clone, Debug)]
pub enum Event {
    Stage(String),
    Done(Verdict),
    Batch(Vec<Verdict>),
    Rechecked(Vec<Rule>, Vec<RuleState>),
    Failed(String),
}

pub fn targets(url: &str) -> Vec<Target> {
    let mut t = picker::custom_targets(&[url.to_string()]);
    t.push(Target { name: picker::host_of(url), category: String::new(), url: url.to_string(), kind: CheckKind::Freeze });
    t
}

fn run_checks(targets: &[Target]) -> Vec<Check> {
    std::thread::scope(|s| {
        let jobs: Vec<_> = targets.iter().map(|t| s.spawn(move || picker::run_check(t))).collect();
        jobs.into_iter().map(|j| j.join().unwrap_or_else(|_| Check::failed())).collect()
    })
}

fn kin(brand: &str, label: &str) -> bool {
    let shared = brand.chars().zip(label.chars()).take_while(|(a, b)| a == b).count();
    shared >= 5 || (brand.len() >= 4 && label.len() >= 4 && (label.contains(brand) || brand.contains(label)))
}

fn add_domain(out: &mut Vec<String>, d: &str) {
    let d = d.trim().trim_end_matches('.').to_lowercase();
    if d.is_empty() || out.len() >= MAX_DOMAINS || out.iter().any(|x| d == *x || d.ends_with(&format!(".{x}"))) {
        return;
    }
    out.retain(|x| !x.ends_with(&format!(".{d}")));
    out.push(d);
}

pub fn pick_domains(url: &str, found: Option<&super::discover::Found>) -> Vec<String> {
    let main = super::discover::base_domain(&picker::host_of(url));
    let brand = main.split('.').next().unwrap_or("").to_string();
    let mut out = Vec::new();
    add_domain(&mut out, &main);
    for d in super::known::service_domains(&main) {
        add_domain(&mut out, d);
    }
    if let Some(found) = found {
        for g in &found.groups {
            let label = g.base.split('.').next().unwrap_or("");
            if g.main || g.related || (!g.third_party && kin(&brand, label)) {
                add_domain(&mut out, &g.base);
                continue;
            }
            for h in &g.hosts {
                if h.split('.').any(|l| kin(&brand, l)) {
                    add_domain(&mut out, h);
                }
            }
        }
    }
    out
}

pub fn site_domains(url: &str) -> Vec<String> {
    pick_domains(url, super::discover::run(url).ok().as_ref())
}

pub fn advise(baseline: &Trial, strategy: Option<&Trial>, techniques: &[Trial]) -> Advice {
    match (baseline.passed(), strategy.map(Trial::passed)) {
        (true, None) | (true, Some(true)) => Advice::Fine,
        (true, Some(false)) => Advice::Exclude,
        (false, Some(true)) => Advice::Works(strategy.map(|s| s.label.clone()).unwrap_or_default()),
        (false, _) => {
            let best = techniques
                .iter()
                .filter(|t| t.ok > 0)
                .max_by_key(|t| (t.passed(), t.ok, std::cmp::Reverse(t.ms)));
            match best.and_then(|t| t.rule.clone()) {
                Some(rule) if best.is_some_and(Trial::passed) => Advice::Use(rule),
                _ => Advice::Nothing,
            }
        }
    }
}

fn measure_engine(engine: &Path, bin: &Path, args: &str, log: &str, targets: &[Target], cancel: &AtomicBool) -> Vec<Check> {
    let mut child = match picker::start_raw(engine, bin, args, log) {
        Ok(c) => c,
        Err(_) => return vec![Check::failed(); targets.len()],
    };
    let checks = match picker::warm_up(&mut child, log, cancel) {
        Ok(()) if !cancel.load(Ordering::Relaxed) => run_checks(targets),
        _ => vec![Check::failed(); targets.len()],
    };
    picker::stop_engine(Some(child));
    checks
}

struct Pause {
    was_running: bool,
    had_divert: bool,
}

fn pause() -> Pause {
    let was_running = service::query(service::SERVICE) == ServiceState::Running;
    let had_divert = service::query("WinDivert") != ServiceState::NotInstalled;
    if was_running {
        super::run_hidden("net", &["stop", service::SERVICE]);
    }
    picker::stop_engine(None);
    Pause { was_running, had_divert }
}

fn resume(p: Pause) -> Option<String> {
    picker::stop_engine(None);
    let restore = p.was_running.then(service::start);
    if !p.was_running && !p.had_divert {
        let _ = service::unload_windivert();
    }
    match restore {
        Some(Err(e)) => Some(e.to_string()),
        _ => None,
    }
}

fn inspect(paths: &Paths, url: String, domains: Vec<String>, current: Option<&Strategy>, gf: &GameFilter, cancel: &AtomicBool, emit: &dyn Fn(Event)) -> Verdict {
    let targets = targets(&url);

    emit(Event::Stage("Проверяю без обхода".into()));
    let base_checks = run_checks(&targets);
    let baseline = Trial::from("Без обхода", &base_checks);

    let mut strategy_trial = None;
    if let Some(s) = current {
        if !cancel.load(Ordering::Relaxed) {
            emit(Event::Stage(format!("Проверяю со стратегией «{}»", s.name)));
            let checks = match strategy::extract_args(s, paths, gf) {
                Ok(args) => measure_engine(&s.engine, &s.bin, &args, &s.name, &targets, cancel),
                Err(_) => vec![Check::failed(); targets.len()],
            };
            strategy_trial = Some(Trial::from(s.name.clone(), &checks));
        }
    }

    let origin_name = current.as_ref().map(|s| s.name.clone()).unwrap_or_default();
    let mut techniques = Vec::new();
    let need = !baseline.passed() && !strategy_trial.as_ref().is_some_and(Trial::passed);
    if need {
        let engine = paths.engine();
        let bin = paths.bin();
        for (i, t) in TECHNIQUES.iter().enumerate() {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            emit(Event::Stage(format!("Приём {} из {}: {}", i + 1, TECHNIQUES.len(), t.name)));
            let args = format!(
                "--wf-tcp=80,443 --filter-tcp=80,443 --hostlist-domains={} {}",
                domains.join(","),
                technique_args(t, &bin)
            );
            let checks = measure_engine(&engine, &bin, &args, &format!("rule-{}", t.id), &targets, cancel);
            let mut trial = Trial::from(t.name, &checks);
            trial.rule = Some(Rule { technique: t.id.to_string(), domains: domains.clone(), origin: origin_name.clone(), ..Default::default() });
            techniques.push(trial);
        }
        if !techniques.iter().any(Trial::passed) {
            let all = strategy::discover(paths);
            let donors: Vec<&Strategy> = picker::optimal(paths, &all)
                .into_iter()
                .filter_map(|i| all.get(i))
                .filter(|s| s.engine == engine && current.as_ref().is_none_or(|c| c.path != s.path))
                .collect();
            for (i, s) in donors.iter().enumerate() {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                emit(Event::Stage(format!("Профиль из стратегии {} из {}: {}", i + 1, donors.len(), s.name)));
                let Ok(rule) = borrow_rule(paths, s, domains.clone(), &origin_name) else { continue };
                let args = format!("--wf-tcp=80,443 --wf-udp=443 {}", profiles(std::slice::from_ref(&rule), &bin));
                let checks = measure_engine(&engine, &bin, &args, &format!("rule-from-{i}"), &targets, cancel);
                let mut trial = Trial::from(format!("Как в «{}»", s.name), &checks);
                trial.rule = Some(rule);
                techniques.push(trial);
            }
        }
    }

    let advice = advise(&baseline, strategy_trial.as_ref(), &techniques);
    Verdict { url, domains, baseline, strategy: strategy_trial, techniques, advice, origin: origin_name }
}

pub fn find(paths: Paths, url: String, current: Option<Strategy>, gf: GameFilter, cancel: &AtomicBool, emit: impl Fn(Event)) {
    if !picker::curl_available() {
        emit(Event::Failed("curl.exe не найден: он входит в Windows 10 1803 и новее".into()));
        return;
    }
    emit(Event::Stage("Ищу домены, с которых сайт грузит картинки и скрипты".into()));
    let domains = site_domains(&url);
    let paused = pause();
    let v = inspect(&paths, url, domains, current.as_ref(), &gf, cancel, &emit);
    let restore = resume(paused);
    if cancel.load(Ordering::Relaxed) {
        emit(Event::Failed("Проверка сайта отменена".into()));
        return;
    }
    if let Some(e) = restore {
        emit(Event::Failed(format!("Проверка закончилась, но службу не удалось запустить: {e}")));
    }
    emit(Event::Done(v));
}

pub fn find_many(paths: Paths, urls: Vec<String>, current: Option<Strategy>, gf: GameFilter, cancel: &AtomicBool, emit: impl Fn(Event)) {
    if !picker::curl_available() {
        emit(Event::Failed("curl.exe не найден: он входит в Windows 10 1803 и новее".into()));
        return;
    }
    emit(Event::Stage("Ищу домены, с которых сайты грузят картинки и скрипты".into()));
    let found: Vec<Vec<String>> = std::thread::scope(|s| {
        let jobs: Vec<_> = urls.iter().map(|u| s.spawn(move || site_domains(u))).collect();
        jobs.into_iter().zip(&urls).map(|(j, u)| j.join().unwrap_or_else(|_| vec![super::discover::base_domain(&picker::host_of(u))])).collect()
    });
    let paused = pause();
    let n = urls.len();
    let mut out = Vec::new();
    for ((i, url), domains) in urls.into_iter().enumerate().zip(found) {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let host = picker::host_of(&url);
        let tag = format!("Сайт {} из {n}, {host}", i + 1);
        let relay = |e: Event| match e {
            Event::Stage(s) => emit(Event::Stage(format!("{tag}: {s}"))),
            other => emit(other),
        };
        out.push(inspect(&paths, url, domains, current.as_ref(), &gf, cancel, &relay));
    }
    let restore = resume(paused);
    if cancel.load(Ordering::Relaxed) {
        emit(Event::Failed("Подбор для сайтов отменён".into()));
        return;
    }
    if let Some(e) = restore {
        emit(Event::Failed(format!("Подбор закончился, но службу не удалось запустить: {e}")));
    }
    emit(Event::Batch(out));
}

pub fn recheck(paths: Paths, list: Vec<Rule>, current: Strategy, gf: GameFilter, cancel: &AtomicBool, emit: impl Fn(Event)) {
    if !picker::curl_available() {
        emit(Event::Failed("curl.exe не найден: он входит в Windows 10 1803 и новее".into()));
        return;
    }
    let args = match strategy::extract_args(&current, &paths, &gf) {
        Ok(a) => a,
        Err(e) => {
            emit(Event::Failed(format!("Не удалось прочитать стратегию «{}»: {e}", current.name)));
            return;
        }
    };
    let was_running = service::query(service::SERVICE) == ServiceState::Running;
    let had_divert = service::query("WinDivert") != ServiceState::NotInstalled;
    if was_running {
        super::run_hidden("net", &["stop", service::SERVICE]);
    }
    picker::stop_engine(None);
    let bin = paths.bin();
    let mut states = Vec::new();
    for (i, rule) in list.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let site = rule.domains.first().cloned().unwrap_or_default();
        emit(Event::Stage(format!("Правило {} из {}: {site}", i + 1, list.len())));
        let targets = targets(&format!("https://{site}/"));
        let plain = measure_engine(&current.engine, &current.bin, &args, &current.name, &targets, cancel);
        let with = inject(&args, std::slice::from_ref(rule), &bin);
        let ruled = measure_engine(&current.engine, &current.bin, &with, &current.name, &targets, cancel);
        states.push(rule_state(&Trial::from("с правилом", &ruled), &Trial::from("без правила", &plain)));
    }
    picker::stop_engine(None);
    let restore = was_running.then(service::start);
    if !was_running && !had_divert {
        let _ = service::unload_windivert();
    }
    if cancel.load(Ordering::Relaxed) {
        emit(Event::Failed("Перепроверка правил отменена".into()));
        return;
    }
    if let Some(Err(e)) = restore {
        emit(Event::Failed(format!("Перепроверка закончилась, но службу не удалось запустить: {e}")));
    }
    emit(Event::Rechecked(list, states));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(t: &str, d: &[&str], o: &str) -> Rule {
        Rule { technique: t.into(), domains: d.iter().map(|s| s.to_string()).collect(), origin: o.into(), ..Default::default() }
    }

    #[test]
    fn domains_include_cdn_and_known_service() {
        use super::super::discover::{Found, Group};
        let g = |base: &str, hosts: &[&str], main: bool, related: bool, third: bool| Group {
            base: base.into(),
            hosts: hosts.iter().map(|h| h.to_string()).collect(),
            ips: vec![],
            main,
            related,
            third_party: third,
            cloudflare: true,
        };
        let found = Found {
            final_url: "https://steamcommunity.com/market/".into(),
            groups: vec![
                g("steamcommunity.com", &["steamcommunity.com"], true, false, false),
                g("steamstatic.com", &["community.cloudflare.steamstatic.com"], false, false, false),
                g("akamaihd.net", &["steamcdn-a.akamaihd.net", "other-a.akamaihd.net"], false, false, true),
                g("google-analytics.com", &["www.google-analytics.com"], false, false, true),
            ],
            assets_scanned: 3,
        };
        let d = pick_domains("https://steamcommunity.com/market/", Some(&found));
        assert_eq!(d[0], "steamcommunity.com");
        for want in ["steampowered.com", "steamstatic.com", "steamcdn-a.akamaihd.net"] {
            assert!(d.iter().any(|x| x == want), "{want}: {d:?}");
        }
        assert!(!d.iter().any(|x| x.contains("google") || x == "akamaihd.net" || x == "other-a.akamaihd.net"), "{d:?}");
        let site = Found {
            final_url: "https://7tv.app/".into(),
            groups: vec![
                g("7tv.app", &["7tv.app"], true, false, false),
                g("7tv.io", &["cdn.7tv.io"], false, true, false),
                g("cloudflare.com", &["cdnjs.cloudflare.com"], false, false, true),
            ],
            assets_scanned: 1,
        };
        assert_eq!(pick_domains("https://7tv.app/", Some(&site)), vec!["7tv.app".to_string(), "7tv.io".to_string()]);
        assert_eq!(pick_domains("https://www.example.org/x", None), vec!["example.org".to_string()]);
    }

    #[test]
    fn borrowed_rules() {
        let b = Rule {
            technique: "from:general (ALT2)".into(),
            domains: vec!["soundcloud.com".into(), "whatsapp.com".into()],
            origin: "general".into(),
            tcp: "--dpi-desync=fake,multisplit --dpi-desync-fake-tls=\"{BIN}tls_clienthello_www_google_com.bin\" --dpi-desync-repeats=6".into(),
            udp: "--dpi-desync=fake --dpi-desync-fake-quic=\"{BIN}quic_initial_www_google_com.bin\"".into(),
        };
        assert_eq!(parse(&render(std::slice::from_ref(&b))), vec![b.clone()]);
        assert_eq!(label(&b.technique), "Как в «general (ALT2)»");
        let out = inject("--wf-tcp=443 --filter-tcp=443 --dpi-desync=fake", std::slice::from_ref(&b), Path::new(r"C:\hz\bin"));
        assert!(out.contains(r#"--filter-tcp=80,443 --hostlist-domains=soundcloud.com,whatsapp.com --dpi-desync=fake,multisplit --dpi-desync-fake-tls="C:\hz\bin\tls_clienthello_www_google_com.bin""#), "{out}");
        assert!(out.contains(r#"--filter-udp=443 --hostlist-domains=soundcloud.com,whatsapp.com --dpi-desync=fake --dpi-desync-fake-quic="C:\hz\bin\quic_initial_www_google_com.bin" --new --filter-udp=443 --hostlist-domains=soundcloud.com,whatsapp.com --new"#), "{out}");
        for evil in [
            "from:x | a.com | g | --debug=@C:\\x.txt |",
            "from:x | a.com | g | --dpi-desync-fake-tls=\"{BIN}..\\..\\secret\" |",
            "from:x | a.com | g | --dpi-desync-fake-tls=C:\\x.bin |",
            "from:x | a.com | g | --hostlist-auto=x.txt |",
            "from:x | a.com | g |  |",
        ] {
            assert!(parse(evil).is_empty(), "{evil}");
        }
        let bin = Path::new(r"C:\Hz Folder\bin");
        let toks = vec!["--dpi-desync=fake".to_string(), r"--dpi-desync-fake-quic=C:\Hz Folder\bin\quic_initial_www_google_com.bin".to_string()];
        assert_eq!(stored(&toks, bin), r#"--dpi-desync=fake --dpi-desync-fake-quic="{BIN}quic_initial_www_google_com.bin""#);
    }

    #[test]
    fn rules_round_trip() {
        let rules = vec![rule(NONE, &["steampowered.com", "steamstatic.com"], "general"), rule("alt3-badseq", &["x.com"], "")];
        let text = render(&rules);
        assert_eq!(parse(&text), rules);
        assert_eq!(parse("bogus | a.com | x\nnone | not a domain | y\n# c\nnone | B.com |"), vec![rule(NONE, &["b.com"], "")]);
        assert_eq!(label(NONE), "Не обходить");
        assert_eq!(label("alt3-badseq"), "ALT3, подмена badseq");
    }

    #[test]
    fn upsert_moves_domains() {
        let mut rules = vec![rule(NONE, &["a.com", "b.com"], "g"), rule("seqovl", &["c.com"], "")];
        upsert(&mut rules, rule("alt3-ts", &["b.com", "c.com"], ""));
        assert_eq!(rules, vec![rule(NONE, &["a.com"], "g"), rule("alt3-ts", &["b.com", "c.com"], "")]);
    }

    #[test]
    fn rules_go_before_strategy_profiles() {
        let bin = Path::new(r"C:\hz\bin");
        let args = "--wf-tcp=80,443 --wf-udp=443 --filter-udp=443 --hostlist=\"x\" --dpi-desync=fake --new --filter-tcp=443 --dpi-desync=fake";
        assert_eq!(inject(args, &[], bin), args);
        let rules = vec![rule(NONE, &["steampowered.com"], "g"), rule("seqovl", &["x.com"], "")];
        let out = inject(args, &rules, bin);
        assert!(out.starts_with(
            "--wf-tcp=80,443 --wf-udp=443 --filter-tcp=80,443 --hostlist-domains=steampowered.com --new --filter-tcp=80,443 --hostlist-domains=x.com --dpi-desync=multisplit"
        ));
        assert!(out.contains(r#"--dpi-desync-split-seqovl-pattern="C:\hz\bin\tls_clienthello_www_google_com.bin""#));
        assert!(out.contains("--filter-udp=443 --hostlist-domains=steampowered.com,x.com --new --filter-udp=443 --hostlist=\"x\""));
        assert_eq!(out.matches("--new").count(), args.matches("--new").count() + 3);
        assert_eq!(inject("--wf-tcp=80 --dpi-desync=fake", &rules, bin), "--wf-tcp=80 --dpi-desync=fake");
    }

    #[test]
    fn rule_states() {
        let t = |ok: usize| Trial { label: "s".into(), rule: None, ok, total: 3, ms: 10 };
        assert_eq!(rule_state(&t(3), &t(0)), RuleState::Works);
        assert_eq!(rule_state(&t(3), &t(3)), RuleState::NotNeeded);
        assert_eq!(rule_state(&t(2), &t(3)), RuleState::Broken);
    }

    #[test]
    fn advice_follows_results() {
        let t = |ok: usize, tech: Option<&str>, ms: u32| Trial {
            label: "s".into(),
            rule: tech.map(|id| rule(id, &["a.com"], "")),
            ok,
            total: 3,
            ms,
        };
        assert_eq!(advise(&t(3, None, 0), None, &[]), Advice::Fine);
        assert_eq!(advise(&t(3, None, 0), Some(&t(0, None, 0)), &[]), Advice::Exclude);
        assert_eq!(advise(&t(0, None, 0), Some(&t(3, None, 0)), &[]), Advice::Works("s".into()));
        let techs = [t(3, Some("a"), 300), t(3, Some("b"), 100), t(2, Some("c"), 50)];
        assert_eq!(advise(&t(0, None, 0), Some(&t(1, None, 0)), &techs), Advice::Use(rule("b", &["a.com"], "")));
        assert_eq!(advise(&t(0, None, 0), None, &[t(2, Some("c"), 50)]), Advice::Nothing);
    }

    #[test]
    #[ignore]
    fn engine_accepts_rules_and_techniques() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf();
        let paths = Paths { root };
        let bin = paths.bin();
        let dry = |args: &str| {
            use std::os::windows::process::CommandExt;
            let out = std::process::Command::new(paths.engine())
                .env("__COMPAT_LAYER", "RunAsInvoker")
                .current_dir(&bin)
                .raw_arg(format!("--dry-run {args}"))
                .creation_flags(0x0800_0000)
                .output()
                .unwrap();
            (out.status.success(), String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr))
        };
        for t in TECHNIQUES {
            let args = format!("--wf-tcp=80,443 --filter-tcp=80,443 --hostlist-domains=a.com {}", technique_args(t, &bin));
            let (ok, log) = dry(&args);
            assert!(ok, "{}: {log}", t.id);
        }
        let all: Vec<Rule> = std::iter::once(rule(NONE, &["steampowered.com"], "g"))
            .chain(TECHNIQUES.iter().enumerate().map(|(i, t)| Rule { technique: t.id.into(), domains: vec![format!("d{i}.com")], ..Default::default() }))
            .collect();
        let mut borrowed = 0;
        for s in strategy::discover(&paths).iter().filter(|s| s.engine == paths.engine()) {
            super::super::lists::ensure_user_files(&s.lists).unwrap();
            for host in ["youtube.com", "discord.com", "x.com"] {
                if let Ok(r) = borrow_rule(&paths, s, vec![host.to_string()], "test") {
                    assert!(r.tcp.is_empty() || r.tcp.split(' ').any(|t| t.starts_with("--dpi-desync=")), "{} {host}: {}", s.name, r.tcp);
                    assert!(r.udp.is_empty() || r.udp.split(' ').any(|t| t.starts_with("--dpi-desync=")), "{} {host}: {}", s.name, r.udp);
                    let args = format!("--wf-tcp=80,443 --wf-udp=443 {}", profiles(std::slice::from_ref(&r), &bin));
                    let (ok, log) = dry(&args);
                    assert!(ok, "{} {host}: {log}", s.name);
                    borrowed += 1;
                }
            }
        }
        assert!(borrowed > 10, "{borrowed}");
        let mut checked = 0;
        for s in strategy::discover(&paths).iter().filter(|s| s.engine == paths.engine()) {
            super::super::lists::ensure_user_files(&s.lists).unwrap();
            let args = inject(&strategy::extract_args(s, &paths, &GameFilter::default()).unwrap(), &all, &bin);
            let (ok, log) = dry(&args);
            assert!(ok, "{}: {log}", s.name);
            checked += 1;
        }
        assert!(checked > 5, "{checked}");
        let exp: Vec<Strategy> = strategy::discover(&paths).into_iter().filter(|s| s.category == crate::core::EXPERIMENTAL).collect();
        assert!(exp.len() >= 11 && exp.iter().all(|s| s.lists == paths.exp_lists() && s.bin == paths.bin()), "{exp:?}");
    }
}
