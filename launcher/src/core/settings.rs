use winreg::RegKey;
use winreg::enums::*;

const KEY: &str = r"Software\HolyZapret";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    pub advanced: bool,
    pub close_to_tray: bool,
    pub sidebar_open: bool,
    pub wizard_done: bool,
    pub flow: bool,
    pub watch: bool,
    pub auto_pick: bool,
    pub auto_update: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { advanced: true, close_to_tray: true, sidebar_open: false, wizard_done: false, flow: true, watch: true, auto_pick: false, auto_update: false }
    }
}

fn read(k: &RegKey, name: &str, default: bool) -> bool {
    k.get_value::<u32, _>(name).map(|v| v != 0).unwrap_or(default)
}

pub fn load() -> Settings {
    let d = Settings::default();
    match RegKey::predef(HKEY_CURRENT_USER).open_subkey(KEY) {
        Ok(k) => Settings {
            advanced: read(&k, "Advanced", d.advanced),
            close_to_tray: read(&k, "CloseToTray", d.close_to_tray),
            sidebar_open: read(&k, "SidebarOpen", d.sidebar_open),
            wizard_done: read(&k, "WizardDone", d.wizard_done),
            flow: read(&k, "Flow", d.flow),
            watch: read(&k, "Watch", d.watch),
            auto_pick: read(&k, "AutoPick", d.auto_pick),
            auto_update: read(&k, "AutoUpdate", d.auto_update),
        },
        Err(_) => d,
    }
}

pub fn last_strategy() -> Option<String> {
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(KEY)
        .ok()
        .and_then(|k| k.get_value::<String, _>("LastStrategy").ok())
        .filter(|v| !v.is_empty())
}

pub fn ray_enabled() -> bool {
    true
}

pub const DEFAULT_EXPERIMENT: &str = "adaptive";

pub fn ray_experiment() -> String {
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(KEY)
        .ok()
        .and_then(|k| k.get_value::<String, _>("RayExperiment").ok())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| DEFAULT_EXPERIMENT.into())
}

pub fn set_ray_experiment(v: &str) {
    if let Ok((k, _)) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(KEY) {
        let _ = k.set_value("RayExperiment", &v);
    }
}

pub fn ray_start() -> String {
    RegKey::predef(HKEY_CURRENT_USER).open_subkey(KEY).ok().and_then(|k| k.get_value::<String, _>("RayStart").ok()).unwrap_or_default()
}

pub fn set_ray_start(v: &str) {
    if let Ok((k, _)) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(KEY) {
        let _ = k.set_value("RayStart", &v);
    }
}

pub fn take_start_migration() -> bool {
    let Ok((k, _)) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(KEY) else { return false };
    if read(&k, "RayStartMigrated", false) {
        return false;
    }
    k.set_value("RayStartMigrated", &1u32).is_ok()
}

pub fn warp_owned() -> Option<String> {
    RegKey::predef(HKEY_CURRENT_USER).open_subkey(KEY).ok()?.get_value::<String, _>("WarpOwned").ok().filter(|v| !v.is_empty())
}

pub fn set_warp_owned(mode: Option<&str>) {
    if let Ok((k, _)) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(KEY) {
        let _ = match mode {
            Some(m) => k.set_value("WarpOwned", &m),
            None => k.delete_value("WarpOwned"),
        };
    }
}

pub fn take_refresh(version: &str) -> bool {
    let Ok((k, _)) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(KEY) else { return false };
    if k.get_value::<String, _>("ServiceRefreshed").is_ok_and(|v| v == version) {
        return false;
    }
    k.set_value("ServiceRefreshed", &version).is_ok()
}

pub const DEFAULT_QUIC: &str = "adaptive";

pub fn ray_quic() -> String {
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(KEY)
        .ok()
        .and_then(|k| k.get_value::<String, _>("RayQuic").ok())
        .unwrap_or_else(|| DEFAULT_QUIC.into())
}

pub fn ray_autolist() -> bool {
    RegKey::predef(HKEY_CURRENT_USER).open_subkey(KEY).ok().is_none_or(|k| read(&k, "RayAutoList", true))
}

pub fn ray_dns() -> bool {
    RegKey::predef(HKEY_CURRENT_USER).open_subkey(KEY).ok().is_none_or(|k| read(&k, "RayDns", true))
}

pub fn set_ray_dns(on: bool) {
    if let Ok((k, _)) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(KEY) {
        let _ = k.set_value("RayDns", &(on as u32));
    }
}

pub fn ray_gateway() -> bool {
    RegKey::predef(HKEY_CURRENT_USER).open_subkey(KEY).ok().is_some_and(|k| read(&k, "RayGateway", false))
}

pub fn ray_warp() -> bool {
    RegKey::predef(HKEY_CURRENT_USER).open_subkey(KEY).ok().is_some_and(|k| read(&k, "RayWarp", false))
}

pub fn set_ray_warp(on: bool) {
    if let Ok((k, _)) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(KEY) {
        let _ = k.set_value("RayWarp", &(on as u32));
    }
}

pub fn ray_warp_force() -> Vec<String> {
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(KEY)
        .ok()
        .and_then(|k| k.get_value::<String, _>("RayWarpForce").ok())
        .map(|v| v.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect())
        .unwrap_or_default()
}

pub fn set_ray_gateway(on: bool) {
    if let Ok((k, _)) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(KEY) {
        let _ = k.set_value("RayGateway", &(on as u32));
    }
}

pub fn set_ray_autolist(on: bool) {
    if let Ok((k, _)) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(KEY) {
        let _ = k.set_value("RayAutoList", &(on as u32));
    }
}

pub fn set_ray_quic(v: &str) {
    if let Ok((k, _)) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(KEY) {
        let _ = k.set_value("RayQuic", &v);
    }
}

pub fn ray_apps() -> (String, String) {
    let k = RegKey::predef(HKEY_CURRENT_USER).open_subkey(KEY).ok();
    let get = |n: &str| k.as_ref().and_then(|k| k.get_value::<String, _>(n).ok()).unwrap_or_default();
    (get("RayAll"), get("RaySkip"))
}

pub fn set_ray_apps(all: &str, skip: &str) {
    if let Ok((k, _)) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(KEY) {
        let _ = k.set_value("RayAll", &all);
        let _ = k.set_value("RaySkip", &skip);
    }
}

pub fn report_place() -> (String, String) {
    let k = RegKey::predef(HKEY_CURRENT_USER).open_subkey(KEY).ok();
    let get = |n: &str| k.as_ref().and_then(|k| k.get_value::<String, _>(n).ok()).unwrap_or_default();
    (get("ReportIsp"), get("ReportCity"))
}

pub fn set_report_place(isp: &str, city: &str) {
    if let Ok((k, _)) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(KEY) {
        let _ = k.set_value("ReportIsp", &isp);
        let _ = k.set_value("ReportCity", &city);
    }
}

pub fn seen_version() -> Option<String> {
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(KEY)
        .ok()
        .and_then(|k| k.get_value::<String, _>("SeenVersion").ok())
        .filter(|v| !v.is_empty())
}

pub fn notified_version() -> Option<String> {
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(KEY)
        .ok()
        .and_then(|k| k.get_value::<String, _>("NotifiedVersion").ok())
        .filter(|v| !v.is_empty())
}

pub fn set_notified_version(v: &str) {
    if let Ok((k, _)) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(KEY) {
        let _ = k.set_value("NotifiedVersion", &v);
    }
}

pub fn set_seen_version(v: &str) {
    if let Ok((k, _)) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(KEY) {
        let _ = k.set_value("SeenVersion", &v);
    }
}

pub fn set_last_strategy(id: &str) {
    if let Ok((k, _)) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(KEY) {
        let _ = k.set_value("LastStrategy", &id);
    }
}

pub fn save(s: &Settings) {
    if let Ok((k, _)) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(KEY) {
        let _ = k.set_value("Advanced", &(s.advanced as u32));
        let _ = k.set_value("CloseToTray", &(s.close_to_tray as u32));
        let _ = k.set_value("SidebarOpen", &(s.sidebar_open as u32));
        let _ = k.set_value("WizardDone", &(s.wizard_done as u32));
        let _ = k.set_value("Flow", &(s.flow as u32));
        let _ = k.set_value("Watch", &(s.watch as u32));
        let _ = k.set_value("AutoPick", &(s.auto_pick as u32));
        let _ = k.set_value("AutoUpdate", &(s.auto_update as u32));
    }
}
