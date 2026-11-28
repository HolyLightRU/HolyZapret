use super::gamefilter::GameFilter;
use super::paths::Paths;
use super::picker::{self, Check, CheckKind, Target};
use super::service::{self, ServiceState};
use super::strategy::Strategy;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

pub const NONE: &str = "none";
pub const BORROW: &str = "from:";
const FILE: &str = "site-rules-user.txt";
const MAX_DOMAINS: usize = 16;

pub const TECHNIQUES: [&str; 9] = ["general", "sni-split", "sni-pool", "ttl", "disorder", "md5sig", "badseq", "overlap-max", "random"];

const LEGACY: [(&str, &str); 11] = [
    ("badsum", "general"),
    ("alt3-ts", "general"),
    ("alt3-badseq", "badseq"),
    ("alt3-md5sig", "md5sig"),
    ("alt3-ts-badseq", "general"),
    ("alt3-autottl", "ttl"),
    ("seqovl", "general"),
    ("seqovl-sniext", "sni-split"),
    ("fakedsplit", "badseq"),
    ("multidisorder", "disorder"),
    ("fake-md5sig", "md5sig"),
];

static FRESH: Mutex<Vec<String>> = Mutex::new(Vec::new());

pub fn label(id: &str) -> String {
    if id == NONE {
        return "Не обходить".into();
    }
    super::ray::experiment_label(id).to_string()
}

fn known(id: &str) -> Option<String> {
    if id == NONE || TECHNIQUES.contains(&id) {
        return Some(id.to_string());
    }
    if id.starts_with(BORROW) {
        return Some("general".into());
    }
    LEGACY.iter().find(|(old, _)| *old == id).map(|(_, new)| new.to_string())
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
        false
    }

    pub fn line(&self) -> String {
        format!("{} | {} | {}", self.technique, self.domains.join(", "), self.origin)
    }
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
            let technique = known(parts.next()?)?;
            let domains: Vec<String> = parts.next()?.split([',', ' ']).filter_map(|d| super::lists::normalize_domain(d).ok()).collect();
            let origin = parts.next().unwrap_or("").to_string();
            (!domains.is_empty()).then_some(Rule { technique, domains, origin, ..Default::default() })
        })
        .collect()
}

pub fn render(rules: &[Rule]) -> String {
    let mut out = String::from("# Правила для сайтов HolyZapret: приём RayOfLight | домены | откуда\r\n");
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
    if let Ok(mut f) = FRESH.lock() {
        f.extend(rule.domains.iter().cloned());
    }
    for r in rules.iter_mut() {
        r.domains.retain(|d| !rule.domains.iter().any(|n| n.eq_ignore_ascii_case(d)));
    }
    rules.retain(|r| !r.domains.is_empty());
    rules.push(rule);
}

pub fn seed_text(memory: &str, rules: &[Rule], fresh: &[String]) -> String {
    let mut lines: Vec<String> = memory.lines().filter(|l| !l.trim().is_empty()).map(String::from).collect();
    for r in rules {
        let tech = if r.technique == NONE { "none" } else { r.technique.as_str() };
        for d in &r.domains {
            let pos = lines.iter().position(|l| l.split('\t').next().is_some_and(|h| h.eq_ignore_ascii_case(d)));
            let force = fresh.iter().any(|f| f.eq_ignore_ascii_case(d));
            match pos {
                Some(i) if force => lines[i] = format!("{d}\t{tech}\t0\t0"),
                Some(_) => {}
                None => lines.push(format!("{d}\t{tech}\t0\t0")),
            }
        }
    }
    lines.iter().map(|l| format!("{l}\n")).collect()
}

pub fn seed_memory(paths: &Paths) {
    let rules = load(paths);
    let fresh = FRESH.lock().map(|mut f| std::mem::take(&mut *f)).unwrap_or_default();
    if rules.is_empty() {
        return;
    }
    let file = super::ray::memory_path(paths);
    let text = seed_text(&fs::read_to_string(&file).unwrap_or_default(), &rules, &fresh);
    let _ = fs::write(&file, text);
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

fn measure_ray(paths: &Paths, experiment: &str, targets: &[Target], gf: &GameFilter, cancel: &AtomicBool) -> Vec<Check> {
    let Some(exe) = super::ray::exe(paths) else { return vec![Check::failed(); targets.len()] };
    let log = format!("rule-{experiment}");
    let live = std::env::temp_dir().join("holyzapret-picker").join(format!("{log}.json"));
    let args = super::ray::cli_args(paths, experiment, &live, gf);
    let mut child = match picker::start_raw(&exe, &paths.bin(), &args, &log) {
        Ok(c) => c,
        Err(_) => return vec![Check::failed(); targets.len()],
    };
    let checks = match picker::warm_up(&mut child, &log, cancel) {
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

fn inspect(paths: &Paths, url: String, domains: Vec<String>, _current: Option<&Strategy>, gf: &GameFilter, cancel: &AtomicBool, emit: &dyn Fn(Event)) -> Verdict {
    let targets = targets(&url);
    emit(Event::Stage("Проверяю без обхода".into()));
    let baseline = Trial::from("Без обхода", &run_checks(&targets));
    let mut techniques = Vec::new();
    if !baseline.passed() {
        for (i, t) in TECHNIQUES.iter().enumerate() {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            emit(Event::Stage(format!("Приём {} из {}: {}", i + 1, TECHNIQUES.len(), label(t))));
            let checks = measure_ray(paths, t, &targets, gf, cancel);
            let mut trial = Trial::from(label(t), &checks);
            trial.rule = Some(Rule { technique: t.to_string(), domains: domains.clone(), origin: "RayOfLight".into(), ..Default::default() });
            let done = trial.passed();
            techniques.push(trial);
            if done {
                break;
            }
        }
    }
    let advice = advise(&baseline, None, &techniques);
    Verdict { url, domains, baseline, strategy: None, techniques, advice, origin: "RayOfLight".into() }
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

pub fn recheck(paths: Paths, list: Vec<Rule>, _current: Strategy, gf: GameFilter, cancel: &AtomicBool, emit: impl Fn(Event)) {
    if !picker::curl_available() {
        emit(Event::Failed("curl.exe не найден: он входит в Windows 10 1803 и новее".into()));
        return;
    }
    let paused = pause();
    let mut states = Vec::new();
    for (i, rule) in list.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let site = rule.domains.first().cloned().unwrap_or_default();
        emit(Event::Stage(format!("Правило {} из {}: {site}", i + 1, list.len())));
        let targets = targets(&format!("https://{site}/"));
        let plain = measure_ray(&paths, "adaptive", &targets, &gf, cancel);
        let tech = if rule.technique == NONE { "none" } else { rule.technique.as_str() };
        let ruled = if tech == "none" { run_checks(&targets) } else { measure_ray(&paths, tech, &targets, &gf, cancel) };
        states.push(rule_state(&Trial::from("с правилом", &ruled), &Trial::from("без правила", &plain)));
    }
    let restore = resume(paused);
    if cancel.load(Ordering::Relaxed) {
        emit(Event::Failed("Перепроверка правил отменена".into()));
        return;
    }
    if let Some(e) = restore {
        emit(Event::Failed(format!("Перепроверка закончилась, но службу не удалось запустить: {e}")));
    }
    emit(Event::Rechecked(list, states));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_rules_map_to_ray_techniques_and_seed_memory() {
        let r = parse("alt3-badseq | rutor.info | general\nfrom:ALT 4 | x.com | g | --dpi-desync=fake |\nmultidisorder | a.org |\nweird | b.org |\n");
        assert_eq!(r.iter().map(|r| r.technique.as_str()).collect::<Vec<_>>(), vec!["badseq", "general", "disorder"]);
        let mem = "rutor.info\tgeneral\t5\t1\nother.com\tttl\t1\t0\n";
        let kept = seed_text(mem, &r, &[]);
        assert!(kept.contains("rutor.info\tgeneral\t5\t1\n") && kept.contains("x.com\tgeneral\t0\t0\n") && kept.contains("other.com\tttl\t1\t0\n"));
        let forced = seed_text(mem, &r, &["rutor.info".into()]);
        assert!(forced.contains("rutor.info\tbadseq\t0\t0\n"));
        assert_eq!(label("sni-split"), "Разрез по имени сайта");
    }

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
    fn rules_round_trip() {
        let rules = vec![rule(NONE, &["steampowered.com", "steamstatic.com"], "general"), rule("badseq", &["x.com"], "")];
        let text = render(&rules);
        assert_eq!(parse(&text), rules);
        assert_eq!(parse("bogus | a.com | x\nnone | not a domain | y\n# c\nnone | B.com |"), vec![rule(NONE, &["b.com"], "")]);
        assert_eq!(label(NONE), "Не обходить");
        assert_eq!(label("badseq"), "Фейк с неверным seq");
    }

    #[test]
    fn upsert_moves_domains() {
        let mut rules = vec![rule(NONE, &["a.com", "b.com"], "g"), rule("seqovl", &["c.com"], "")];
        upsert(&mut rules, rule("alt3-ts", &["b.com", "c.com"], ""));
        assert_eq!(rules, vec![rule(NONE, &["a.com"], "g"), rule("alt3-ts", &["b.com", "c.com"], "")]);
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

}
