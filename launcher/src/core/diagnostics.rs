use super::flowseal;
use super::paths::Paths;
use super::service::{self, ServiceState};
use super::{kill_process, process_running, run_hidden};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Level {
    Ok,
    Warn,
    Bad,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fix {
    EnableTimestamps,
    UnloadWinDivert,
    RemoveServices(Vec<String>),
    RemoveZapret,
    StopEngines,
}

impl Fix {
    pub fn label(&self) -> &'static str {
        match self {
            Fix::EnableTimestamps => "Включить",
            Fix::UnloadWinDivert => "Выгрузить",
            Fix::RemoveServices(_) => "Удалить",
            Fix::RemoveZapret => "Удалить службу",
            Fix::StopEngines => "Остановить",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Finding {
    pub level: Level,
    pub title: String,
    pub hint: String,
    pub link: Option<String>,
    pub fix: Option<Fix>,
}

const CONFLICTING: [&str; 4] = ["GoodbyeDPI", "discordfix_zapret", "winws1", "winws2"];

fn ok(title: &str) -> Finding {
    Finding { level: Level::Ok, title: title.into(), hint: String::new(), link: None, fix: None }
}

fn issue(level: Level, title: impl Into<String>, hint: impl Into<String>) -> Finding {
    Finding { level, title: title.into(), hint: hint.into(), link: None, fix: None }
}

impl Finding {
    fn link(mut self, url: &str) -> Self {
        self.link = Some(url.into());
        self
    }

    fn fix(mut self, f: Fix) -> Self {
        self.fix = Some(f);
        self
    }
}

pub fn parse_services(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut name: Option<String> = None;
    for line in text.lines() {
        let l = line.trim();
        if let Some(v) = l.strip_prefix("SERVICE_NAME:") {
            name = Some(v.trim().to_string());
        } else if let Some(v) = l.strip_prefix("DISPLAY_NAME:") {
            if let Some(n) = name.take() {
                out.push((n, v.trim().to_string()));
            }
        }
    }
    out
}

pub fn parse_probe(text: &str) -> (Option<bool>, usize) {
    let mut ts = None;
    let mut doh = 0;
    for line in text.lines() {
        if let Some(v) = line.trim().strip_prefix("TS=") {
            let v = v.trim().to_lowercase();
            if !v.is_empty() {
                ts = Some(v == "enabled");
            }
        } else if let Some(v) = line.trim().strip_prefix("DOH=") {
            doh = v.trim().parse().unwrap_or(0);
        }
    }
    (ts, doh)
}

pub fn netsh_timestamps(text: &str) -> Option<bool> {
    text.lines()
        .find(|l| l.to_lowercase().contains("timestamps"))
        .and_then(|l| l.rsplit(':').next())
        .map(|v| v.trim().eq_ignore_ascii_case("enabled"))
}

pub fn hosts_mention_youtube(text: &str) -> bool {
    text.lines().map(str::trim).filter(|l| !l.starts_with('#')).any(|l| {
        let l = l.to_lowercase();
        l.contains("youtube.com") || l.contains("youtu.be")
    })
}

pub fn has_cyrillic(p: &Path) -> bool {
    p.to_string_lossy().chars().any(|c| matches!(c, 'а'..='я' | 'А'..='Я' | 'ё' | 'Ё'))
}

pub fn in_folder(p: &Path, folder: &Path) -> bool {
    let a = p.to_string_lossy().to_lowercase().replace('/', "\\");
    let mut b = folder.to_string_lossy().to_lowercase().replace('/', "\\");
    if b.is_empty() {
        return false;
    }
    if !b.ends_with('\\') {
        b.push('\\');
    }
    format!("{a}\\").starts_with(&b)
}

fn probe() -> (Option<bool>, usize) {
    let script = "$t=(Get-NetTCPSetting -SettingName Internet -ErrorAction SilentlyContinue).Timestamps; \"TS=$t\"; \
        $d=(Get-ChildItem -Recurse -Path 'HKLM:System\\CurrentControlSet\\Services\\Dnscache\\InterfaceSpecificParameters\\' -ErrorAction SilentlyContinue | \
        Get-ItemProperty | Where-Object { $_.DohFlags -gt 0 } | Measure-Object).Count; \"DOH=$d\"";
    let (_, out) = run_hidden("powershell", &["-NoProfile", "-Command", script]);
    let (mut ts, doh) = parse_probe(&out);
    if ts.is_none() {
        ts = netsh_timestamps(&run_hidden("netsh", &["interface", "tcp", "show", "global"]).1);
    }
    (ts, doh)
}

fn onedrive_folders() -> Vec<PathBuf> {
    ["OneDrive", "OneDriveConsumer", "OneDriveCommercial"]
        .iter()
        .filter_map(|k| std::env::var_os(k))
        .map(PathBuf::from)
        .collect()
}

pub fn run(paths: &Paths) -> Vec<Finding> {
    let mut v = Vec::new();
    let (ts, doh) = probe();
    let active = parse_services(&run_hidden("sc", &["query"]).1);
    let has = |needle: &str| {
        let n = needle.to_lowercase();
        active.iter().any(|(s, d)| s.to_lowercase().contains(&n) || d.to_lowercase().contains(&n))
    };

    v.push(if run_hidden("sc", &["query", "BFE"]).1.contains("RUNNING") {
        ok("Base Filtering Engine работает")
    } else {
        issue(Level::Bad, "Base Filtering Engine не запущена", "Без этой службы Windows движок не работает. Включите её в services.msc")
    });

    v.push(match ts {
        Some(true) => ok("TCP timestamps включены"),
        Some(false) => issue(Level::Bad,
            "TCP timestamps выключены",
            "Стратегии с --dpi-desync-fooling=ts без них не работают",
        )
        .fix(Fix::EnableTimestamps),
        None => issue(Level::Warn, "Не удалось узнать состояние TCP timestamps", "Проверьте вручную: netsh interface tcp show global"),
    });

    let inet = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
        .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Internet Settings")
        .ok();
    let proxy_on = inet.as_ref().and_then(|k| k.get_value::<u32, _>("ProxyEnable").ok()).unwrap_or(0) == 1;
    v.push(if proxy_on {
        let server = inet.as_ref().and_then(|k| k.get_value::<String, _>("ProxyServer").ok()).unwrap_or_default();
        issue(Level::Warn, format!("Включён системный прокси {server}"), "Убедитесь, что он рабочий, или выключите, если не пользуетесь прокси")
    } else {
        ok("Системный прокси выключен")
    });

    v.push(if doh > 0 {
        ok("Защищённый DNS настроен")
    } else {
        issue(Level::Warn,
            "Защищённый DNS не настроен",
            "Включите DNS через HTTPS в браузере или в параметрах сети Windows 11 у стороннего DNS-провайдера",
        )
    });

    let hosts = std::env::var("SystemRoot").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(r"C:\Windows"));
    let hosts_text = std::fs::read_to_string(hosts.join(r"System32\drivers\etc\hosts")).unwrap_or_default();
    v.push(if hosts_mention_youtube(&hosts_text) {
        issue(Level::Warn, "В файле hosts есть записи youtube.com или youtu.be", "Они могут мешать работе YouTube")
    } else {
        ok("Файл hosts не трогает YouTube")
    });

    v.push(if has_cyrillic(&paths.root) {
        issue(Level::Warn,
            "В пути к HolyZapret есть кириллица",
            "Если обход не работает, перенесите папку, например в C:\\zapret",
        )
    } else {
        ok("Путь без кириллицы")
    });

    v.push(if onedrive_folders().iter().any(|f| in_folder(&paths.root, f)) {
        issue(Level::Bad, "HolyZapret лежит в папке OneDrive", "Перенесите папку, например в C:\\zapret")
    } else {
        ok("Папка не в OneDrive")
    });

    let mut sys_dirs = vec![paths.bin()];
    if flowseal::installed(paths).is_some() {
        sys_dirs.push(flowseal::dir(paths).join("bin"));
    }
    let missing: Vec<String> = sys_dirs
        .iter()
        .filter(|d| !d.join("WinDivert64.sys").is_file())
        .map(|d| d.strip_prefix(&paths.root).unwrap_or(d).display().to_string())
        .collect();
    v.push(if missing.is_empty() {
        ok("WinDivert64.sys на месте")
    } else {
        issue(Level::Bad, format!("Нет WinDivert64.sys в {}", missing.join(", ")), "Его мог удалить антивирус: верните файл и добавьте папку в исключения")
    });

    v.push(if process_running("AdguardSvc.exe") {
        issue(Level::Bad, "Найден Adguard", "Может ломать Discord")
            .link("https://github.com/Flowseal/zapret-discord-youtube/issues/417")
    } else {
        ok("Adguard не найден")
    });
    v.push(if has("killer") {
        issue(Level::Bad, "Найдены службы Killer", "Killer конфликтует с zapret, отключите или удалите его")
            .link("https://github.com/Flowseal/zapret-discord-youtube/issues/2512#issuecomment-2821119513")
    } else {
        ok("Killer не найден")
    });
    let intel = active.iter().any(|(s, d)| {
        let t = format!("{s} {d}").to_lowercase();
        t.contains("intel") && t.contains("connectivity") && t.contains("network")
    });
    v.push(if intel {
        issue(Level::Bad, "Найдена Intel Connectivity Network Service", "Она конфликтует с zapret")
            .link("https://github.com/ValdikSS/GoodbyeDPI/issues/541#issuecomment-2661670982")
    } else {
        ok("Intel Connectivity не найдена")
    });
    v.push(if has("tracsrvwrapper") || has("epwd") {
        issue(Level::Bad, "Найден Check Point", "Он конфликтует с zapret, попробуйте его удалить")
    } else {
        ok("Check Point не найден")
    });
    v.push(if has("smartbyte") {
        issue(Level::Bad, "Найден SmartByte", "Удалите или отключите его в services.msc")
    } else {
        ok("SmartByte не найден")
    });

    let vpn: Vec<String> = active
        .iter()
        .filter(|(s, d)| {
            let t = format!("{s} {d}").to_lowercase();
            t.contains("vpn") || t.contains("happservice")
        })
        .map(|(s, _)| s.clone())
        .collect();
    v.push(if vpn.is_empty() {
        ok("VPN-службы не запущены")
    } else {
        issue(Level::Warn, format!("Запущены VPN-службы: {}", vpn.join(", ")), "Часть VPN конфликтует с zapret, выключите их")
    });

    let conflicts: Vec<String> = CONFLICTING.iter().filter(|s| run_hidden("sc", &["query", s]).0).map(|s| s.to_string()).collect();
    v.push(if conflicts.is_empty() {
        ok("Других обходов не найдено")
    } else {
        issue(Level::Bad,
            format!("Найдены службы других обходов: {}", conflicts.join(", ")),
            "Они держат WinDivert и мешают HolyZapret",
        )
        .fix(Fix::RemoveServices(conflicts))
    });

    let st = service::status(paths);
    if let Some(d) = st.foreign_dir() {
        v.push(
            issue(
                Level::Warn,
                format!("Служба zapret установлена из другой папки: {}", d.display()),
                "Скорее всего это оригинальный Flowseal. Удалите её, чтобы поставить стратегию HolyZapret, или оставьте, если пользуетесь им",
            )
            .fix(Fix::RemoveZapret),
        );
    }
    let service_dir = st.foreign_dir().map(Path::to_path_buf);
    let manual: Vec<String> = st
        .foreign_engines
        .iter()
        .filter(|d| Some(*d) != service_dir.as_ref())
        .map(|d| d.display().to_string())
        .collect();
    v.push(if manual.is_empty() {
        ok("Других запущенных обходов нет")
    } else {
        issue(
            Level::Bad,
            format!("Запущен другой обход из {}", manual.join(", ")),
            "Два движка одновременно мешают друг другу. Остановите его или закройте его окно",
        )
        .fix(Fix::StopEngines)
    });
    let engine = st.engine_running;
    let divert = st.windivert != ServiceState::NotInstalled;
    v.push(if divert && !engine {
        issue(Level::Warn,
            "WinDivert загружен, но движок не запущен",
            "Драйвер остался от прошлого запуска или от другой программы",
        )
        .fix(Fix::UnloadWinDivert)
    } else {
        ok("WinDivert в порядке")
    });
    v
}

pub fn apply(fix: &Fix) -> anyhow::Result<String> {
    match fix {
        Fix::EnableTimestamps => {
            let (ok, out) = run_hidden("netsh", &["interface", "tcp", "set", "global", "timestamps=enabled"]);
            if !ok {
                anyhow::bail!("{}", out.trim());
            }
            Ok("TCP timestamps включены".into())
        }
        Fix::UnloadWinDivert => service::unload_windivert().map(|_| "Драйвер WinDivert выгружен".into()),
        Fix::RemoveZapret => Ok(service::remove()),
        Fix::StopEngines => {
            service::stop_engines();
            Ok("Движки остановлены".into())
        }
        Fix::RemoveServices(names) => {
            let mut failed = Vec::new();
            for n in names {
                run_hidden("net", &["stop", n]);
                if !run_hidden("sc", &["delete", n]).0 {
                    failed.push(n.clone());
                }
            }
            for d in ["WinDivert", "WinDivert14"] {
                run_hidden("net", &["stop", d]);
                run_hidden("sc", &["delete", d]);
            }
            if failed.is_empty() {
                Ok(format!("Удалены службы: {}", names.join(", ")))
            } else {
                anyhow::bail!("не удалось удалить: {}", failed.join(", "))
            }
        }
    }
}

const DISCORDS: [(&str, &str, &str); 4] = [
    ("discord", "Discord.exe", "Discord"),
    ("discordptb", "DiscordPTB.exe", "Discord PTB"),
    ("discordcanary", "DiscordCanary.exe", "Discord Canary"),
    ("discorddevelopment", "DiscordDevelopment.exe", "Discord Development"),
];

pub fn clear_discord_cache() -> anyhow::Result<String> {
    let appdata = PathBuf::from(std::env::var("APPDATA")?);
    let mut cleared = Vec::new();
    let mut failed = Vec::new();
    for (dir, exe, label) in DISCORDS {
        let base = appdata.join(dir);
        if !base.is_dir() {
            continue;
        }
        if process_running(exe) {
            kill_process(exe);
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        let mut ok = true;
        for d in ["Cache", "Code Cache", "GPUCache"] {
            let p = base.join(d);
            if p.exists() && std::fs::remove_dir_all(&p).is_err() {
                ok = false;
            }
        }
        if ok { cleared.push(label) } else { failed.push(label) }
    }
    if cleared.is_empty() && failed.is_empty() {
        anyhow::bail!("Discord не найден");
    }
    if !failed.is_empty() {
        anyhow::bail!("не удалось очистить {}", failed.join(", "));
    }
    Ok(format!("Кэш очищен: {}", cleared.join(", ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn services_parsed() {
        let text = "\r\nSERVICE_NAME: BFE\r\nDISPLAY_NAME: Base Filtering Engine\r\n        TYPE : 20\r\n\r\nSERVICE_NAME: NordVPN\r\nDISPLAY_NAME: NordVPN Service\r\n";
        let s = parse_services(text);
        assert_eq!(s, vec![("BFE".into(), "Base Filtering Engine".into()), ("NordVPN".into(), "NordVPN Service".into())]);
    }

    #[test]
    fn probe_and_netsh_parsed() {
        assert_eq!(parse_probe("TS=Enabled\r\nDOH=2\r\n"), (Some(true), 2));
        assert_eq!(parse_probe("TS=Disabled\nDOH=0\n"), (Some(false), 0));
        assert_eq!(parse_probe("TS=\nDOH=\n"), (None, 0));
        assert_eq!(netsh_timestamps("RFC 1323 Timestamps                 : disabled \r\n"), Some(false));
        assert_eq!(netsh_timestamps("RFC 1323 Timestamps : enabled"), Some(true));
    }

    #[test]
    fn hosts_and_paths() {
        assert!(hosts_mention_youtube("1.2.3.4 www.youtube.com\n"));
        assert!(!hosts_mention_youtube("# 1.2.3.4 youtube.com\n127.0.0.1 localhost\n"));
        assert!(has_cyrillic(Path::new(r"C:\Пользователи\zapret")));
        assert!(!has_cyrillic(Path::new(r"C:\zapret")));
        assert!(in_folder(Path::new(r"C:\Users\a\OneDrive\zapret"), Path::new(r"C:\Users\a\OneDrive")));
        assert!(!in_folder(Path::new(r"C:\Users\a\OneDriveX\zapret"), Path::new(r"C:\Users\a\OneDrive")));
        assert!(!in_folder(Path::new(r"C:\zapret"), Path::new("")));
    }
}

