use super::gamefilter::GameFilter;
use super::paths::Paths;
use super::picker::{self, Check, CheckKind, Target};
use super::service::{self, ServiceState};
use super::strategy::{self, Strategy};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

pub const NONE: &str = "none";
const FILE: &str = "site-rules-user.txt";
const MAX_DOMAINS: usize = 8;

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
    technique(id).map(|t| t.name.to_string()).unwrap_or_else(|| id.to_string())
}

pub fn technique_args(t: &Technique, bin: &Path) -> String {
    let bin = format!("{}\\", bin.display());
    t.args.replace("{PATTERN}", PATTERN).replace("{BIN}", &bin)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    pub technique: String,
    pub domains: Vec<String>,
    pub origin: String,
}

pub fn path(paths: &Paths) -> PathBuf {
    paths.root.join("utils").join(FILE)
}

pub fn parse(text: &str) -> Vec<Rule> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let mut parts = l.splitn(3, '|').map(str::trim);
            let technique = parts.next()?.to_string();
            let domains: Vec<String> = parts
                .next()?
                .split([',', ' '])
                .filter_map(|d| super::lists::normalize_domain(d).ok())
                .collect();
            let origin = parts.next().unwrap_or("").to_string();
            let known = technique == NONE || self::technique(&technique).is_some();
            (known && !domains.is_empty()).then_some(Rule { technique, domains, origin })
        })
        .collect()
}

pub fn render(rules: &[Rule]) -> String {
    let mut out = String::from("# Правила для сайтов HolyZapret: приём | домены | для какой стратегии подобрано\r\n");
    for r in rules {
        out.push_str(&format!("{} | {} | {}\r\n", r.technique, r.domains.join(", "), r.origin));
    }
    out
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
    for r in rules {
        let domains = r.domains.join(",");
        let body = match technique(&r.technique) {
            Some(t) => format!(" {}", technique_args(t, bin)),
            None => String::new(),
        };
        out.push(format!("--filter-tcp=80,443 --hostlist-domains={domains}{body}"));
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
    pub technique: Option<String>,
    pub ok: usize,
    pub total: usize,
    pub ms: u32,
}

impl Trial {
    fn from(label: impl Into<String>, technique: Option<String>, checks: &[Check]) -> Trial {
        let ok: Vec<&Check> = checks.iter().filter(|c| c.ok).collect();
        let ms = if ok.is_empty() { 0 } else { ok.iter().map(|c| c.ms).sum::<u32>() / ok.len() as u32 };
        Trial { label: label.into(), technique, ok: ok.len(), total: checks.len(), ms }
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
    Use(String),
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

#[derive(Clone, Debug)]
pub enum Event {
    Stage(String),
    Done(Verdict),
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

fn related_domains(url: &str) -> Vec<String> {
    let main = super::discover::base_domain(&picker::host_of(url));
    let mut out = vec![main.clone()];
    if let Ok(found) = super::discover::run(url) {
        for g in found.groups.iter().filter(|g| (g.main || g.related) && !g.third_party && !g.cloudflare) {
            if !out.contains(&g.base) && out.len() < MAX_DOMAINS {
                out.push(g.base.clone());
            }
        }
    }
    out
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
            match best.and_then(|t| t.technique.clone()) {
                Some(id) if best.is_some_and(Trial::passed) => Advice::Use(id),
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

pub fn find(paths: Paths, url: String, current: Option<Strategy>, gf: GameFilter, cancel: &AtomicBool, emit: impl Fn(Event)) {
    if !picker::curl_available() {
        emit(Event::Failed("curl.exe не найден: он входит в Windows 10 1803 и новее".into()));
        return;
    }
    let was_running = service::query(service::SERVICE) == ServiceState::Running;
    let had_divert = service::query("WinDivert") != ServiceState::NotInstalled;
    if was_running {
        super::run_hidden("net", &["stop", service::SERVICE]);
    }
    picker::stop_engine(None);
    let targets = targets(&url);

    emit(Event::Stage("Проверяю без обхода".into()));
    let base_checks = run_checks(&targets);
    let baseline = Trial::from("Без обхода", None, &base_checks);
    let domains = if baseline.passed() {
        emit(Event::Stage("Ищу связанные домены".into()));
        related_domains(&url)
    } else {
        vec![super::discover::base_domain(&picker::host_of(&url))]
    };

    let mut strategy_trial = None;
    if let Some(s) = &current {
        if !cancel.load(Ordering::Relaxed) {
            emit(Event::Stage(format!("Проверяю со стратегией «{}»", s.name)));
            let checks = match strategy::extract_args(s, &paths, &gf) {
                Ok(args) => measure_engine(&s.engine, &s.bin, &args, &s.name, &targets, cancel),
                Err(_) => vec![Check::failed(); targets.len()],
            };
            strategy_trial = Some(Trial::from(s.name.clone(), None, &checks));
        }
    }

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
            techniques.push(Trial::from(t.name, Some(t.id.to_string()), &checks));
        }
    }

    picker::stop_engine(None);
    let restore = was_running.then(service::start);
    if !was_running && !had_divert {
        let _ = service::unload_windivert();
    }
    if cancel.load(Ordering::Relaxed) {
        emit(Event::Failed("Проверка сайта отменена".into()));
        return;
    }
    if let Some(Err(e)) = restore {
        emit(Event::Failed(format!("Проверка закончилась, но службу не удалось запустить: {e}")));
    }
    let advice = advise(&baseline, strategy_trial.as_ref(), &techniques);
    let origin = current.map(|s| s.name).unwrap_or_default();
    emit(Event::Done(Verdict { url, domains, baseline, strategy: strategy_trial, techniques, advice, origin }));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(t: &str, d: &[&str], o: &str) -> Rule {
        Rule { technique: t.into(), domains: d.iter().map(|s| s.to_string()).collect(), origin: o.into() }
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
    fn advice_follows_results() {
        let t = |ok: usize, tech: Option<&str>, ms: u32| Trial { label: "s".into(), technique: tech.map(String::from), ok, total: 3, ms };
        assert_eq!(advise(&t(3, None, 0), None, &[]), Advice::Fine);
        assert_eq!(advise(&t(3, None, 0), Some(&t(0, None, 0)), &[]), Advice::Exclude);
        assert_eq!(advise(&t(0, None, 0), Some(&t(3, None, 0)), &[]), Advice::Works("s".into()));
        let techs = [t(3, Some("a"), 300), t(3, Some("b"), 100), t(2, Some("c"), 50)];
        assert_eq!(advise(&t(0, None, 0), Some(&t(1, None, 0)), &techs), Advice::Use("b".into()));
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
            .chain(TECHNIQUES.iter().enumerate().map(|(i, t)| Rule { technique: t.id.into(), domains: vec![format!("d{i}.com")], origin: String::new() }))
            .collect();
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
