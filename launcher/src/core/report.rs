use super::diagnostics::{Finding, Level};
use super::picker::{self, CheckKind, Outcome, Target};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

pub struct Input<'a> {
    pub when: String,
    pub version: String,
    pub launcher: String,
    pub game: String,
    pub root: &'a Path,
    pub targets: &'a [Target],
    pub ordered: Vec<&'a Outcome>,
    pub noisy: Vec<String>,
    pub isp: String,
    pub city: String,
    pub problem: String,
    pub unreachable: Vec<bool>,
    pub diag: &'a [Finding],
    pub sections: Vec<(String, Vec<String>)>,
    pub log: Vec<String>,
}

fn kind(k: CheckKind) -> &'static str {
    match k {
        CheckKind::Tls12 => "TLS 1.2",
        CheckKind::Tls13 => "TLS 1.3",
        CheckKind::Freeze => "загрузка",
        CheckKind::Quic => "QUIC",
    }
}

pub fn windows_name() -> String {
    let k = winreg::RegKey::predef(winreg::enums::HKEY_LOCAL_MACHINE).open_subkey(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion").ok();
    let get = |n: &str| k.as_ref().and_then(|k| k.get_value::<String, _>(n).ok()).unwrap_or_default();
    let build = picker::windows_build();
    let mut product = get("ProductName");
    if build >= 22000 {
        product = product.replace("Windows 10", "Windows 11");
    }
    format!("{product} {} (сборка {build})", get("DisplayVersion")).trim().to_string()
}

fn outcome_block(out: &mut String, place: Option<usize>, o: &Outcome, input: &Input) {
    let (ok, total) = o.score();
    let title = match place {
        Some(n) => format!("{n}. {}", o.name),
        None => o.name.clone(),
    };
    let id = o
        .path
        .as_ref()
        .map(|p| p.strip_prefix(input.root).unwrap_or(p).display().to_string())
        .map(|s| format!(" [{s}]"))
        .unwrap_or_default();
    let _ = writeln!(out, "\n== {title}{id}: {ok}/{total}, {} мс ==", o.avg_ms());
    if let Some(e) = &o.error {
        let _ = writeln!(out, "Не запустилась: {e}");
        return;
    }
    let cats: Vec<String> = picker::categories(input.targets)
        .iter()
        .map(|c| {
            let (a, b) = o.category(input.targets, c);
            (b > 0).then(|| format!("{c} {a}/{b}"))
        })
        .flatten()
        .collect();
    let _ = writeln!(out, "{}", cats.join(" | "));
    if let Some(r) = &o.ray {
        let _ = writeln!(out, "{r}");
    }
    let skipped: Vec<String> = input.targets.iter().zip(&o.checks).filter(|(_, c)| !c.counted()).map(|(t, _)| t.name.clone()).collect();
    if !skipped.is_empty() {
        let _ = writeln!(out, "Не обходит (не открываются и без обхода): {}", skipped.join(", "));
    }
    let failed: Vec<String> = input
        .targets
        .iter()
        .zip(&o.checks)
        .enumerate()
        .filter(|(_, (_, c))| !c.ok && !c.skipped)
        .map(|(i, (t, c))| {
            let mark = if input.unreachable.get(i).copied().unwrap_or(false) { ", недоступно везде" } else { "" };
            format!("  - {} / {} ({}): {}{mark}", t.category, t.name, kind(t.kind), c.detail())
        })
        .collect();
    if failed.is_empty() {
        let _ = writeln!(out, "Все проверки пройдены");
    } else {
        let _ = writeln!(out, "Не прошли:");
        for f in failed {
            let _ = writeln!(out, "{f}");
        }
    }
}

pub fn build(input: &Input) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "Отчёт HolyZapret");
    let _ = writeln!(out, "Дата: {}", input.when);
    let build = if super::TEST_BUILD { ", тестовая сборка" } else { "" };
    let _ = writeln!(out, "HolyZapret: {} (лаунчер {}{build})", input.version, input.launcher);
    let _ = writeln!(out, "Windows: {}", windows_name());
    let _ = writeln!(out, "TLS 1.3 в проверках: {}", if picker::tls13_supported() { "да" } else { "по ответу сайта (в curl этой Windows нет TLS 1.3)" });
    let _ = writeln!(out, "Игровой фильтр: {}", input.game);
    let or_blank = |s: &str| if s.trim().is_empty() { "(впишите сами)".to_string() } else { s.trim().to_string() };
    let _ = writeln!(out, "Провайдер: {}", or_blank(&input.isp));
    let _ = writeln!(out, "Город: {}", or_blank(&input.city));
    let _ = writeln!(out, "Что не работает (голос Discord, игры, видео, сайты): {}", or_blank(&input.problem.replace(['\r', '\n'], " ")));
    if !input.noisy.is_empty() {
        let _ = writeln!(out, "Не учитывались при выборе лучшей (случайный успех): {}", input.noisy.join(", "));
    }
    let dead: Vec<String> = input
        .targets
        .iter()
        .zip(&input.unreachable)
        .filter(|(_, u)| **u)
        .map(|(t, _)| format!("{} / {} ({})", t.category, t.name, kind(t.kind)))
        .collect();
    if !dead.is_empty() {
        let _ = writeln!(out, "Недоступны и без обхода, и со всеми приёмами (похоже на блокировку по IP): {}", dead.join(", "));
    }
    for (title, lines) in &input.sections {
        let _ = writeln!(out, "\n== {title} ==");
        if lines.is_empty() {
            let _ = writeln!(out, "нет");
        }
        for l in lines {
            let _ = writeln!(out, "{l}");
        }
    }
    if input.ordered.is_empty() {
        let _ = writeln!(out, "\n== Подбор ==\nв этом запуске лаунчера не запускался");
    }
    let mut place = 0;
    for o in &input.ordered {
        if o.path.is_some() {
            place += 1;
            outcome_block(&mut out, Some(place), o, input);
        } else {
            outcome_block(&mut out, None, o, input);
        }
    }
    let _ = writeln!(out, "\n== Диагностика ==");
    if input.diag.is_empty() {
        let _ = writeln!(out, "не запускалась");
    }
    for f in input.diag {
        let mark = match f.level {
            Level::Ok => "OK ",
            Level::Warn => "?  ",
            Level::Bad => "ERR",
        };
        let _ = writeln!(out, "{mark} {}", f.title);
    }
    if !input.log.is_empty() {
        let _ = writeln!(out, "\n== Журнал, последние записи ==");
        for l in &input.log {
            let _ = writeln!(out, "{l}");
        }
    }
    out
}

pub fn desktop() -> PathBuf {
    winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
        .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Explorer\Shell Folders")
        .ok()
        .and_then(|k| k.get_value::<String, _>("Desktop").ok())
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .unwrap_or_else(|| PathBuf::from(std::env::var("USERPROFILE").unwrap_or_default()).join("Desktop"))
}

pub fn file_name(when: &str) -> String {
    let safe: String = when.chars().map(|c| if c == ':' || c == ' ' { '-' } else { c }).collect();
    format!("HolyZapret-отчёт-{safe}.txt")
}

pub fn save(text: &str, when: &str) -> anyhow::Result<PathBuf> {
    let dir = desktop();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(file_name(when));
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(text.replace('\n', "\r\n").as_bytes());
    std::fs::write(&path, bytes)?;
    Ok(path)
}

pub fn reveal(path: &Path) {
    use std::os::windows::process::CommandExt;
    let _ = std::process::Command::new("explorer.exe").raw_arg(format!("/select,\"{}\"", path.display())).spawn();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::picker::{Check, parse_targets};

    #[test]
    fn report_lists_failures_by_target() {
        let targets = parse_targets("### Discord\nDiscordMain = \"https://discord.com\"\n");
        let c = |ok, exit, code| Check { ok, frozen: false, ms: 100, code, exit, skipped: false };
        let base = Outcome { name: "Без обхода".into(), path: None, error: None, checks: vec![c(false, 28, 0), c(false, 35, 0)], ray: None };
        let root = PathBuf::from(r"C:\hz");
        let best = Outcome {
            name: "general".into(),
            path: Some(root.join("general.bat")),
            error: None,
            checks: vec![c(true, 0, 200), c(false, 35, 0)],
            ray: Some("RayOfLight: обработано 2".into()),
        };
        let broken = Outcome { name: "x".into(), path: Some(root.join("x.bat")), error: Some("could not read".into()), checks: vec![], ray: None };
        let input = Input {
            when: "22.11 01:10".into(),
            version: "3.0.0".into(),
            launcher: "3.0.0".into(),
            game: "Выключен".into(),
            root: &root,
            targets: &targets,
            ordered: vec![&base, &best, &broken],
            noisy: vec!["Хостинги / CDN77".into()],
            isp: "Ростелеком".into(),
            city: String::new(),
            problem: "голос\nDiscord".into(),
            unreachable: vec![false, true],
            diag: &[],
            sections: vec![("Состояние".into(), vec!["Служба: работает, стратегия general".into()]), ("Правила для сайтов".into(), vec![])],
            log: vec!["10:00 [ОК] Служба установлена".into()],
        };
        let r = build(&input);
        assert!(r.contains("== Без обхода: 0/2"));
        assert!(r.contains("Discord 1/2
RayOfLight: обработано 2
"));
        assert!(r.contains("(случайный успех): Хостинги / CDN77"));
        assert!(r.contains("Провайдер: Ростелеком\nГород: (впишите сами)\n"));
        assert!(r.contains("(голос Discord, игры, видео, сайты): голос Discord\n"));
        assert!(r.contains("\n== Состояние ==\nСлужба: работает, стратегия general\n"));
        assert!(r.contains("\n== Правила для сайтов ==\nнет\n"));
        assert!(r.ends_with("== Журнал, последние записи ==\n10:00 [ОК] Служба установлена\n"));
        let empty = Input { ordered: vec![], ..input };
        assert!(build(&empty).contains("== Подбор ==\nв этом запуске лаунчера не запускался"));
        assert!(r.contains("(похоже на блокировку по IP): Discord / DiscordMain (TLS 1.3)"));
        assert!(r.contains("Discord / DiscordMain (TLS 1.3): ошибка TLS, недоступно везде"));
        assert!(r.contains("Discord / DiscordMain (TLS 1.2): таймаут\n"));
        assert!(r.contains("== 1. general [general.bat]: 1/2"));
        assert!(r.contains("Discord / DiscordMain (TLS 1.3): ошибка TLS"));
        assert!(r.contains("Discord / DiscordMain (TLS 1.2): таймаут"));
        assert!(r.contains("== 2. x [x.bat]"));
        assert!(r.contains("Не запустилась: could not read"));
        assert_eq!(file_name("22.11 01:10"), "HolyZapret-отчёт-22.11-01-10.txt");
    }
}
