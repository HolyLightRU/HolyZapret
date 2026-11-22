use super::{Config, DEFAULT_PORT, Handle, hex, new_secret, parse_secret};
use crate::core::procs::{self, Proc};
use std::sync::Mutex;
use std::sync::atomic::Ordering;
use winreg::RegKey;
use winreg::enums::HKEY_CURRENT_USER;

const KEY: &str = r"Software\HolyZapret";
static PROXY: Mutex<Option<Handle>> = Mutex::new(None);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Saved {
    pub enabled: bool,
    pub config: Config,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub port: u16,
    pub active: usize,
    pub total: usize,
    pub via_ws: usize,
    pub via_cf: usize,
    pub via_tcp: usize,
    pub failed: usize,
    pub bad_secret: usize,
    pub up: u64,
    pub down: u64,
    pub last_error: String,
    pub last_cf: String,
}

pub fn load() -> Saved {
    let k = RegKey::predef(HKEY_CURRENT_USER).open_subkey(KEY).ok();
    let num = |n: &str| k.as_ref().and_then(|k| k.get_value::<u32, _>(n).ok());
    let text = |n: &str| k.as_ref().and_then(|k| k.get_value::<String, _>(n).ok()).unwrap_or_default();
    let secret = parse_secret(&text("TgSecret"));
    let config = Config {
        port: num("TgPort").and_then(|p| u16::try_from(p).ok()).filter(|p| *p > 0).unwrap_or(DEFAULT_PORT),
        secret: secret.unwrap_or_else(new_secret),
        cloudflare: num("TgCloudflare").is_none_or(|v| v != 0),
        cf_domain: text("TgCfDomain"),
    };
    let saved = Saved { enabled: num("TgProxy").is_some_and(|v| v != 0), config };
    if secret.is_none() {
        save(&saved);
    }
    saved
}

pub fn save(s: &Saved) {
    if let Ok((k, _)) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(KEY) {
        let _ = k.set_value("TgProxy", &(s.enabled as u32));
        let _ = k.set_value("TgPort", &(s.config.port as u32));
        let _ = k.set_value("TgSecret", &hex(&s.config.secret));
        let _ = k.set_value("TgCloudflare", &(s.config.cloudflare as u32));
        let _ = k.set_value("TgCfDomain", &s.config.cf_domain);
    }
}

pub fn start(config: Config) -> anyhow::Result<()> {
    stop();
    let h = super::start(config)?;
    *PROXY.lock().unwrap() = Some(h);
    Ok(())
}

pub fn stop() {
    let h = PROXY.lock().unwrap().take();
    drop(h);
}

pub fn running() -> bool {
    PROXY.lock().unwrap().is_some()
}

pub fn snapshot() -> Option<Snapshot> {
    let g = PROXY.lock().unwrap();
    let h = g.as_ref()?;
    let s = &h.stats;
    let n = |a: &std::sync::atomic::AtomicUsize| a.load(Ordering::Relaxed);
    Some(Snapshot {
        port: h.config.port,
        active: n(&s.active),
        total: n(&s.total),
        via_ws: n(&s.via_ws),
        via_cf: n(&s.via_cf),
        via_tcp: n(&s.via_tcp),
        failed: n(&s.failed),
        bad_secret: n(&s.bad_secret),
        up: s.up.load(Ordering::Relaxed),
        down: s.down.load(Ordering::Relaxed),
        last_error: s.last_error.lock().unwrap().clone(),
        last_cf: s.last_cf.lock().unwrap().clone(),
    })
}

pub fn enable(on: bool) -> anyhow::Result<String> {
    let mut saved = load();
    if !on {
        stop();
        saved.enabled = false;
        save(&saved);
        return Ok("Telegram-прокси выключен".into());
    }
    if let Err(e) = start(saved.config.clone()) {
        let hint = if foreign().is_empty() { String::new() } else { ". Запущен оригинальный TgWsProxy, закройте его".to_string() };
        anyhow::bail!("{e}{hint}");
    }
    saved.enabled = true;
    save(&saved);
    Ok(format!("Telegram-прокси работает на 127.0.0.1:{}", saved.config.port))
}

pub fn apply(config: Config) -> anyhow::Result<()> {
    let mut saved = load();
    saved.config = config;
    save(&saved);
    if running() {
        start(saved.config)?;
    }
    Ok(())
}

pub fn autostart() -> Option<anyhow::Result<String>> {
    let saved = load();
    if !saved.enabled || running() {
        return None;
    }
    Some(start(saved.config.clone()).map(|_| format!("Telegram-прокси запущен на 127.0.0.1:{}", saved.config.port)))
}

pub fn is_original(name: &str) -> bool {
    name.to_ascii_lowercase().starts_with("tgwsproxy")
}

pub fn foreign() -> Vec<Proc> {
    procs::find(is_original)
}

pub fn close_foreign() -> usize {
    let found = foreign();
    let mut names: Vec<String> = found.iter().map(|p| p.name.clone()).collect();
    names.dedup();
    names.iter().filter(|n| crate::core::kill_process(n)).count()
}

pub fn open_in_telegram(link: &str) {
    use std::os::windows::process::CommandExt;
    let _ = std::process::Command::new("explorer.exe").raw_arg(format!("\"{link}\"")).spawn();
}

fn handler_exe(command: &str) -> Option<std::path::PathBuf> {
    let c = command.trim();
    let path = match c.strip_prefix('"') {
        Some(rest) => rest.split('"').next()?,
        None => c.split_whitespace().next()?,
    };
    Some(std::path::PathBuf::from(path))
}

pub fn desktop_client() -> Option<String> {
    use winreg::enums::HKEY_LOCAL_MACHINE;
    for root in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        let cmd = RegKey::predef(root)
            .open_subkey(r"Software\Classes\tg\shell\open\command")
            .and_then(|k| k.get_value::<String, _>(""));
        if let Some(exe) = cmd.ok().as_deref().and_then(handler_exe).filter(|p| p.is_file()) {
            return exe.file_stem().map(|n| n.to_string_lossy().into_owned());
        }
    }
    let appdata = std::env::var("APPDATA").unwrap_or_default();
    if std::path::Path::new(&appdata).join("Telegram Desktop").join("Telegram.exe").is_file() {
        return Some("Telegram".into());
    }
    let local = std::env::var("LOCALAPPDATA").unwrap_or_default();
    let store = std::fs::read_dir(std::path::Path::new(&local).join("Packages"))
        .map(|rd| rd.flatten().any(|e| e.file_name().to_string_lossy().starts_with("TelegramMessengerLLP.TelegramDesktop")))
        .unwrap_or(false);
    if store {
        return Some("Telegram".into());
    }
    procs::running("Telegram.exe").then(|| "Telegram".into())
}

pub fn human(bytes: u64) -> String {
    match bytes {
        b if b >= 1 << 30 => format!("{:.1} ГБ", b as f64 / (1u64 << 30) as f64),
        b if b >= 1 << 20 => format!("{:.1} МБ", b as f64 / (1u64 << 20) as f64),
        b if b >= 1 << 10 => format!("{} КБ", b >> 10),
        b => format!("{b} Б"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_original_builds() {
        for n in ["TgWsProxy_windows.exe", "tgwsproxy_windows_7_64bit.exe", "TgWsProxy.exe"] {
            assert!(is_original(n), "{n}");
        }
        assert!(!is_original("Telegram.exe"));
        assert_eq!(handler_exe(r#""C:\Apps\AyuGram\AyuGram.exe"  -- "%1""#), Some(r"C:\Apps\AyuGram\AyuGram.exe".into()));
        assert_eq!(handler_exe(r"C:\T\Telegram.exe -- %1"), Some(r"C:\T\Telegram.exe".into()));
        assert_eq!(human(512), "512 Б");
        assert_eq!(human(3 << 20), "3.0 МБ");
    }
}
