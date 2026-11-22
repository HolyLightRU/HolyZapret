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
}

impl Default for Settings {
    fn default() -> Self {
        Self { advanced: true, close_to_tray: true, sidebar_open: true, wizard_done: false, flow: true }
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
    }
}
