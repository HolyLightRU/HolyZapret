use super::{DEFAULT_PORT, Handle};
use crate::core::run_hidden;
use std::sync::Mutex;
use std::sync::atomic::Ordering;
use winreg::RegKey;
use winreg::enums::HKEY_CURRENT_USER;

const KEY: &str = r"Software\HolyZapret";
const RULE: &str = "HolyZapret LAN proxy";
static PROXY: Mutex<Option<Handle>> = Mutex::new(None);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Saved {
    pub enabled: bool,
    pub port: u16,
}

pub fn load() -> Saved {
    let k = RegKey::predef(HKEY_CURRENT_USER).open_subkey(KEY).ok();
    let num = |n: &str| k.as_ref().and_then(|k| k.get_value::<u32, _>(n).ok());
    Saved {
        enabled: num("LanProxy").is_some_and(|v| v != 0),
        port: num("LanPort").and_then(|p| u16::try_from(p).ok()).filter(|p| *p >= 1024).unwrap_or(DEFAULT_PORT),
    }
}

pub fn save(s: &Saved) {
    if let Ok((k, _)) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(KEY) {
        let _ = k.set_value("LanProxy", &(s.enabled as u32));
        let _ = k.set_value("LanPort", &(s.port as u32));
    }
}

fn firewall(port: Option<u16>) {
    run_hidden("netsh", &["advfirewall", "firewall", "delete", "rule", &format!("name={RULE}")]);
    if let Some(p) = port {
        run_hidden(
            "netsh",
            &[
                "advfirewall", "firewall", "add", "rule", &format!("name={RULE}"), "dir=in", "action=allow", "protocol=TCP",
                &format!("localport={p}"), "remoteip=localsubnet", "profile=any",
            ],
        );
        if let Ok(exe) = std::env::current_exe() {
            run_hidden(
                "netsh",
                &[
                    "advfirewall", "firewall", "add", "rule", &format!("name={RULE}"), "dir=in", "action=allow", "protocol=UDP",
                    &format!("program={}", exe.display()), "remoteip=localsubnet", "profile=any",
                ],
            );
        }
    }
}

pub fn running() -> bool {
    PROXY.lock().unwrap().as_ref().is_some_and(|h| h.stats.sharing.load(Ordering::Relaxed))
}

pub fn address() -> Option<String> {
    let port = PROXY.lock().unwrap().as_ref().filter(|h| h.stats.sharing.load(Ordering::Relaxed))?.port;
    Some(format!("{}:{port}", super::lan_ip()?))
}

fn start(port: u16) -> anyhow::Result<()> {
    if let Some(h) = PROXY.lock().unwrap().as_ref().filter(|h| h.port == port) {
        h.stats.sharing.store(true, Ordering::Relaxed);
        return Ok(());
    }
    shutdown();
    let h = super::start(port)?;
    firewall(Some(port));
    *PROXY.lock().unwrap() = Some(h);
    Ok(())
}

pub fn shutdown() {
    let h = PROXY.lock().unwrap().take();
    if h.is_some() {
        firewall(None);
    }
    drop(h);
}

fn soft_off() {
    if let Some(h) = PROXY.lock().unwrap().as_ref() {
        h.stats.sharing.store(false, Ordering::Relaxed);
    }
}

pub fn enable(on: bool) -> anyhow::Result<String> {
    let mut s = load();
    if !on {
        soft_off();
        s.enabled = false;
        save(&s);
        return Ok("Раздача на телефон выключена. Пока лаунчер открыт, телефоны с автонастройкой (PAC) идут в интернет напрямую".into());
    }
    if super::lan_ip().is_none() {
        anyhow::bail!("компьютер не подключён к домашней сети: нет локального адреса");
    }
    start(s.port)?;
    s.enabled = true;
    save(&s);
    Ok(format!("Раздача на телефон включена: {}", address().unwrap_or_default()))
}

pub fn set_port(port: u16) -> anyhow::Result<()> {
    let mut s = load();
    s.port = port;
    save(&s);
    if running() {
        shutdown();
        start(port)?;
    }
    Ok(())
}

pub fn autostart() -> Option<anyhow::Result<String>> {
    let s = load();
    if !s.enabled || running() {
        return None;
    }
    Some(start(s.port).map(|_| format!("Раздача на телефон запущена: {}", address().unwrap_or_default())))
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub active: usize,
    pub total: usize,
    pub failed: usize,
    pub clients: usize,
    pub up: u64,
    pub down: u64,
    pub last_error: String,
}

pub fn snapshot() -> Option<Snapshot> {
    let g = PROXY.lock().unwrap();
    let s = &g.as_ref().filter(|h| h.stats.sharing.load(Ordering::Relaxed))?.stats;
    Some(Snapshot {
        active: s.active.load(Ordering::Relaxed),
        total: s.total.load(Ordering::Relaxed),
        failed: s.failed.load(Ordering::Relaxed),
        clients: s.clients.lock().unwrap().len(),
        up: s.up.load(Ordering::Relaxed),
        down: s.down.load(Ordering::Relaxed),
        last_error: s.last_error.lock().unwrap().clone(),
    })
}
