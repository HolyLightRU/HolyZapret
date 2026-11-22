use super::{Config, DEFAULT_PORT, Handle, hex, new_secret, parse_secret};
use crate::core::procs::{self, Proc};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
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

const CLIENTS: [&str; 6] = ["Telegram.exe", "AyuGram.exe", "Kotatogram.exe", "64Gram.exe", "materialgram.exe", "Forkgram.exe"];
const INSTALLED: [(&str, &str); 4] = [
    ("Telegram Desktop", "Telegram.exe"),
    ("AyuGram Desktop", "AyuGram.exe"),
    ("Kotatogram Desktop", "Kotatogram.exe"),
    ("64Gram Desktop", "64Gram.exe"),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Client {
    pub name: String,
    pub exe: Option<PathBuf>,
}

impl Client {
    fn from_exe(exe: PathBuf) -> Client {
        let name = exe.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Telegram".into());
        Client { name, exe: Some(exe) }
    }
}

fn is_client(name: &str) -> bool {
    CLIENTS.iter().any(|c| c.eq_ignore_ascii_case(name))
}

fn running_client() -> Option<PathBuf> {
    procs::find(is_client).into_iter().find_map(|p| p.path)
}

fn plain(p: &Path) -> String {
    p.to_string_lossy().trim_start_matches(r"\\?\").to_lowercase()
}

fn same_file(a: &Path, b: &Path) -> bool {
    plain(a) == plain(b)
}

fn handler() -> Option<PathBuf> {
    use winreg::enums::HKEY_LOCAL_MACHINE;
    [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE].into_iter().find_map(|root| {
        let cmd = RegKey::predef(root)
            .open_subkey(r"Software\Classes\tg\shell\open\command")
            .ok()?
            .get_value::<String, _>("")
            .ok()?;
        handler_exe(&cmd).filter(|p| p.is_file())
    })
}

fn remembered() -> Option<PathBuf> {
    let k = RegKey::predef(HKEY_CURRENT_USER).open_subkey(KEY).ok()?;
    let p = PathBuf::from(k.get_value::<String, _>("TgClient").ok()?);
    p.is_file().then_some(p)
}

fn remember(exe: &Path) {
    if remembered().is_some_and(|p| same_file(&p, exe)) {
        return;
    }
    if let Ok((k, _)) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(KEY) {
        let _ = k.set_value("TgClient", &exe.to_string_lossy().into_owned());
    }
}

pub fn desktop_client() -> Option<Client> {
    if let Some(exe) = running_client() {
        remember(&exe);
        return Some(Client::from_exe(exe));
    }
    if let Some(exe) = remembered().or_else(handler) {
        return Some(Client::from_exe(exe));
    }
    let appdata = PathBuf::from(std::env::var("APPDATA").unwrap_or_default());
    if let Some(exe) = INSTALLED.iter().map(|(dir, exe)| appdata.join(dir).join(exe)).find(|p| p.is_file()) {
        return Some(Client::from_exe(exe));
    }
    let local = std::env::var("LOCALAPPDATA").unwrap_or_default();
    let store = std::fs::read_dir(Path::new(&local).join("Packages"))
        .map(|rd| rd.flatten().any(|e| e.file_name().to_string_lossy().starts_with("TelegramMessengerLLP.TelegramDesktop")))
        .unwrap_or(false);
    store.then(|| Client { name: "Telegram".into(), exe: None })
}

fn shell_open(target: &OsStr) {
    use std::os::windows::process::CommandExt;
    let mut arg = OsString::from("\"");
    arg.push(target);
    arg.push("\"");
    let _ = std::process::Command::new("explorer.exe").raw_arg(arg).spawn();
}

fn forward(exe: &Path, link: &str) -> bool {
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("--").arg(link);
    if let Some(dir) = exe.parent() {
        cmd.current_dir(dir);
    }
    cmd.spawn().is_ok()
}

fn wait_running(exe: &Path, secs: u64) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    while std::time::Instant::now() < deadline {
        if running_client().is_some_and(|p| same_file(&p, exe)) {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    false
}

pub fn open_in_telegram(link: &str, client: Option<&Client>) {
    if running_client().is_some_and(|exe| forward(&exe, link)) {
        return;
    }
    let exe = client.and_then(|c| c.exe.clone()).filter(|p| p.is_file());
    match exe {
        Some(exe) if handler().is_none_or(|h| !same_file(&h, &exe)) => {
            shell_open(exe.as_os_str());
            if wait_running(&exe, 20) {
                std::thread::sleep(std::time::Duration::from_secs(2));
                forward(&exe, link);
            }
        }
        _ => shell_open(OsStr::new(link)),
    }
}

fn handler_exe(command: &str) -> Option<PathBuf> {
    let c = command.trim();
    let path = match c.strip_prefix('"') {
        Some(rest) => rest.split('"').next()?,
        None => c.split_whitespace().next()?,
    };
    Some(PathBuf::from(path))
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
        assert!(is_client("ayugram.exe") && is_client("Telegram.exe") && !is_client("TgWsProxy.exe"));
        assert!(same_file(Path::new(r"\\?\C:\Users\я\Desktop\⠀\AyuGram.exe"), Path::new(r"C:\Users\я\Desktop\⠀\ayugram.exe")));
        assert_eq!(Client::from_exe(PathBuf::from(r"C:\Users\я\Desktop\⠀\AyuGram.exe")).name, "AyuGram");
        assert_eq!(handler_exe(r#""C:\Apps\AyuGram\AyuGram.exe"  -- "%1""#), Some(r"C:\Apps\AyuGram\AyuGram.exe".into()));
        assert_eq!(handler_exe(r"C:\T\Telegram.exe -- %1"), Some(r"C:\T\Telegram.exe".into()));
        assert_eq!(human(512), "512 Б");
        assert_eq!(human(3 << 20), "3.0 МБ");
    }
}
