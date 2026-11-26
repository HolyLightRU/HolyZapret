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
    RemoveKnown(Vec<super::known::Found>),
    MoveFolder,
}

impl Fix {
    pub fn label(&self) -> &'static str {
        match self {
            Fix::EnableTimestamps => "Включить",
            Fix::UnloadWinDivert => "Выгрузить",
            Fix::RemoveServices(_) => "Удалить",
            Fix::RemoveZapret => "Удалить службу",
            Fix::StopEngines => "Остановить",
            Fix::RemoveKnown(_) => "Убрать",
            Fix::MoveFolder => "Перенести",
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
    let line = text.lines().find(|l| l.contains("1323") || l.to_lowercase().contains("timestamps"))?;
    match line.rsplit(':').next()?.trim().to_lowercase().as_str() {
        "enabled" => Some(true),
        "disabled" => Some(false),
        _ => None,
    }
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

fn timestamps() -> Option<bool> {
    netsh_timestamps(&run_hidden("netsh", &["interface", "tcp", "show", "global"]).1).or_else(|| {
        let script = "$t=(Get-NetTCPSetting -SettingName Internet -ErrorAction SilentlyContinue).Timestamps; \"TS=$t\"";
        parse_probe(&run_hidden("powershell", &["-NoProfile", "-Command", script]).1).0
    })
}

fn doh_count(key: &winreg::RegKey, depth: u32) -> usize {
    let flags = key.get_value::<u64, _>("DohFlags").ok().or_else(|| key.get_value::<u32, _>("DohFlags").ok().map(u64::from));
    let here = usize::from(flags.is_some_and(|f| f > 0));
    if depth == 0 {
        return here;
    }
    here + key.enum_keys().flatten().filter_map(|n| key.open_subkey(n).ok()).map(|k| doh_count(&k, depth - 1)).sum::<usize>()
}

fn doh_servers() -> usize {
    winreg::RegKey::predef(winreg::enums::HKEY_LOCAL_MACHINE)
        .open_subkey(r"System\CurrentControlSet\Services\Dnscache\InterfaceSpecificParameters")
        .map(|k| doh_count(&k, 6))
        .unwrap_or(0)
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
    let (ts, doh, services, bfe, conflicts, st) = std::thread::scope(|s| {
        let ts = s.spawn(timestamps);
        let doh = s.spawn(doh_servers);
        let services = s.spawn(|| run_hidden("sc", &["query"]).1);
        let bfe = s.spawn(|| run_hidden("sc", &["query", "BFE"]).1);
        let conflicts = s.spawn(|| {
            CONFLICTING.iter().filter(|x| run_hidden("sc", &["query", x]).0).map(|x| x.to_string()).collect::<Vec<String>>()
        });
        let st = s.spawn(|| service::status(paths));
        (
            ts.join().ok().flatten(),
            doh.join().unwrap_or(0),
            services.join().unwrap_or_default(),
            bfe.join().unwrap_or_default(),
            conflicts.join().unwrap_or_default(),
            st.join().ok(),
        )
    });
    let st = st.unwrap_or_else(|| service::status(paths));
    let active = parse_services(&services);
    let has = |needle: &str| {
        let n = needle.to_lowercase();
        active.iter().any(|(s, d)| s.to_lowercase().contains(&n) || d.to_lowercase().contains(&n))
    };

    v.push(if bfe.contains("RUNNING") {
        ok("Base Filtering Engine работает")
    } else {
        issue(Level::Bad, "Base Filtering Engine не запущена", "Без этой службы Windows движок не работает. Включите её в services.msc")
    });

    v.push(match ts {
        Some(true) => ok("TCP timestamps включены"),
        Some(false) => issue(Level::Bad,
            "TCP timestamps выключены",
            "Без них основные приёмы RayOfLight работают хуже: движку приходится брать запасной приём",
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
    } else if super::settings::ray_dns() {
        ok("RayOfLight сам чинит DNS для сайтов из списков")
    } else {
        issue(Level::Warn,
            "Защищённый DNS не настроен",
            "Включите «Чинить DNS для сайтов из списков» в карточке RayOfLight во вкладке «Инструменты» или DNS через HTTPS в браузере",
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
            "Движок может не найти свои файлы. Лаунчер перенесёт папку сам: скопирует её, переустановит службу и автозапуск и перезапустится из новой папки",
        )
        .fix(Fix::MoveFolder)
    } else {
        ok("Путь без кириллицы")
    });

    v.push(if onedrive_folders().iter().any(|f| in_folder(&paths.root, f)) {
        issue(Level::Bad, "HolyZapret лежит в папке OneDrive", "Перенесите папку, например в C:\\zapret")
    } else {
        ok("Папка не в OneDrive")
    });

    let sys_dirs = [paths.bin()];
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

    v.push(if conflicts.is_empty() {
        ok("Других обходов не найдено")
    } else {
        issue(Level::Bad,
            format!("Найдены службы других обходов: {}", conflicts.join(", ")),
            "Они держат WinDivert и мешают HolyZapret",
        )
        .fix(Fix::RemoveServices(conflicts))
    });

    if let Some(d) = st.foreign_dir() {
        v.push(
            issue(
                Level::Warn,
                format!("Служба zapret установлена из другой папки: {}", d.display()),
                "Скорее всего это другая сборка zapret. Удалите её, чтобы включить защиту HolyZapret, или оставьте, если пользуетесь ей",
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
    let known = super::known::in_user_lists(paths);
    if !known.is_empty() {
        let all: Vec<String> = known.iter().flat_map(|f| f.domains.clone()).collect();
        v.push(
            issue(
                Level::Warn,
                format!("В «Моих доменах» есть {}", super::known::names(&all).join(", ")),
                "Эти сервисы обычно работают без обхода, и обход может их сломать. Если какой-то из них не открывается, проверьте его в «Правилах для сайтов» на вкладке «Сайты»",
            )
            .fix(Fix::RemoveKnown(known)),
        );
    }
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
        Fix::RemoveZapret => service::remove().map_err(|e| anyhow::anyhow!(e)),
        Fix::StopEngines => {
            service::stop_engines();
            Ok("Движки остановлены".into())
        }
        Fix::MoveFolder => anyhow::bail!("перенос папки запускается из лаунчера"),
        Fix::RemoveKnown(found) => {
            let n = super::known::remove(found)?;
            let mut msg = format!("Убрано из «Моих доменов»: {n}");
            if service::query(service::SERVICE) == ServiceState::Running {
                match service::restart() {
                    Ok(()) => msg.push_str(", служба перезапущена"),
                    Err(e) => msg.push_str(&format!(", но службу не удалось перезапустить: {e}")),
                }
            }
            Ok(msg)
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
        assert_eq!(netsh_timestamps("Метки времени RFC 1323           : disabled\r\n"), Some(false));
        assert_eq!(netsh_timestamps("Метки времени RFC 1323 : включено"), None);
        assert_eq!(netsh_timestamps("Receive-Side Scaling State : enabled"), None);
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

