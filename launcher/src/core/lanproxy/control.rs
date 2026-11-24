use super::{DEFAULT_PORT, Handle};
use crate::core::run_hidden;
use std::sync::Mutex;
use std::sync::atomic::Ordering;
use winreg::RegKey;
use winreg::enums::HKEY_CURRENT_USER;

const KEY: &str = r"Software\HolyZapret";
const RULE: &str = "HolyZapret LAN proxy";
static PROXY: Mutex<Option<Handle>> = Mutex::new(None);
static FIREWALL_ERROR: Mutex<Option<String>> = Mutex::new(None);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetProfile {
    pub public: bool,
    pub index: u32,
    pub name: String,
}

fn powershell(cmd: &str) -> (bool, String) {
    let full = format!("[Console]::OutputEncoding = [Text.Encoding]::UTF8; {cmd}");
    run_hidden("powershell", &["-NoProfile", "-NonInteractive", "-Command", &full])
}

fn parse_profile(out: &str) -> Option<NetProfile> {
    let line = out.lines().map(str::trim).rfind(|l| !l.is_empty())?;
    let mut it = line.splitn(3, '|');
    let category = it.next()?.trim();
    let index = it.next()?.trim().parse().ok()?;
    let name = it.next().unwrap_or("").trim().to_string();
    Some(NetProfile { public: category.eq_ignore_ascii_case("Public"), index, name })
}

pub fn net_profile() -> Option<NetProfile> {
    let ip = super::lan_ip()?;
    let (ok, out) = powershell(&format!(
        "$p = Get-NetIPAddress -IPAddress '{ip}' -ErrorAction Stop | Get-NetConnectionProfile -ErrorAction Stop | Select-Object -First 1; $p.NetworkCategory.ToString() + '|' + $p.InterfaceIndex + '|' + $p.Name"
    ));
    if ok { parse_profile(&out) } else { None }
}

pub fn make_private(index: u32) -> anyhow::Result<()> {
    let (ok, out) = powershell(&format!("Set-NetConnectionProfile -InterfaceIndex {index} -NetworkCategory Private -ErrorAction Stop"));
    if ok {
        Ok(())
    } else {
        anyhow::bail!("{}", out.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("Windows не разрешил сменить тип сети"))
    }
}

pub fn repair_firewall() -> anyhow::Result<()> {
    let port = PROXY.lock().unwrap().as_ref().map(|h| h.port).ok_or_else(|| anyhow::anyhow!("раздача выключена"))?;
    firewall(Some(port));
    match firewall_error() {
        Some(e) => anyhow::bail!("{e}"),
        None => Ok(()),
    }
}

pub fn firewall_error() -> Option<String> {
    FIREWALL_ERROR.lock().unwrap().clone()
}

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
    *FIREWALL_ERROR.lock().unwrap() = None;
    if let Some(p) = port {
        let (ok, out) = run_hidden(
            "netsh",
            &[
                "advfirewall", "firewall", "add", "rule", &format!("name={RULE}"), "dir=in", "action=allow", "protocol=TCP",
                &format!("localport={p}"), "remoteip=localsubnet", "profile=any",
            ],
        );
        if !ok {
            let why = out.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" ");
            *FIREWALL_ERROR.lock().unwrap() = Some(if why.is_empty() { "netsh завершился с ошибкой".into() } else { why });
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_line_parsed() {
        assert_eq!(parse_profile("\r\nPublic|12|Сеть\r\n"), Some(NetProfile { public: true, index: 12, name: "Сеть".into() }));
        assert_eq!(parse_profile("Private|7|Home|Wi-Fi"), Some(NetProfile { public: false, index: 7, name: "Home|Wi-Fi".into() }));
        assert_eq!(parse_profile("ошибка"), None);
    }

    #[test]
    #[ignore]
    fn live_profile() {
        println!("{:?}", net_profile());
    }
}
