use super::paths::Paths;
use super::strategy::{self, Strategy};
use super::{kill_process, process_running, run_hidden};
use std::os::windows::process::CommandExt;
use std::process::Command;
use winreg::RegKey;
use winreg::enums::*;

pub const SERVICE: &str = "zapret";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServiceState {
    Running,
    Stopped,
    NotInstalled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Owner {
    Ours,
    Foreign(std::path::PathBuf),
    Unknown,
}

#[derive(Clone, Debug)]
pub struct Status {
    pub service: ServiceState,
    pub windivert: ServiceState,
    pub engine_running: bool,
    pub installed_strategy: Option<String>,
    pub game: super::gamefilter::GameFilter,
    pub owner: Owner,
    pub foreign_engines: Vec<std::path::PathBuf>,
}

impl Status {
    pub fn active_strategy(&self) -> Option<&str> {
        if self.service != ServiceState::NotInstalled && self.owner == Owner::Ours {
            self.installed_strategy.as_deref()
        } else {
            None
        }
    }

    pub fn foreign_dir(&self) -> Option<&std::path::Path> {
        match &self.owner {
            Owner::Foreign(d) if self.service != ServiceState::NotInstalled => Some(d),
            _ => None,
        }
    }
}

pub fn query(name: &str) -> ServiceState {
    let (ok, out) = run_hidden("sc", &["query", name]);
    if !ok {
        return ServiceState::NotInstalled;
    }
    if out.contains("RUNNING") { ServiceState::Running } else { ServiceState::Stopped }
}

pub fn status(paths: &Paths) -> Status {
    use super::procs;
    let key = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey(r"System\CurrentControlSet\Services\zapret").ok();
    let installed_strategy = key.as_ref().and_then(|k| k.get_value::<String, _>("zapret-discord-youtube").ok());
    let image = key.as_ref().and_then(|k| k.get_value::<String, _>("ImagePath").ok());
    let service_exe = image.as_deref().and_then(procs::exe_from_command);
    let owner = match &service_exe {
        Some(p) if procs::under(p, &paths.root) => Owner::Ours,
        Some(p) => Owner::Foreign(procs::install_dir(p)),
        None => Owner::Unknown,
    };
    let running = procs::list(&procs::ENGINES);
    let mut foreign_engines: Vec<std::path::PathBuf> = Vec::new();
    for p in running.iter().filter_map(|p| p.path.as_ref()) {
        if !procs::under(p, &paths.root) {
            let d = procs::install_dir(p);
            if !foreign_engines.contains(&d) {
                foreign_engines.push(d);
            }
        }
    }
    let windivert = match query("WinDivert") {
        ServiceState::NotInstalled => query("WinDivert14"),
        s => s,
    };
    Status {
        service: query(SERVICE),
        windivert,
        engine_running: !running.is_empty(),
        installed_strategy,
        game: super::gamefilter::load(paths),
        owner,
        foreign_engines,
    }
}

pub fn stop_engines() {
    kill_process("HolyZapret.exe");
    kill_process("winws.exe");
}


fn command(paths: &Paths, s: &Strategy) -> anyhow::Result<String> {
    let args = super::rules::with_rules(paths, s, &super::gamefilter::load(paths))?;
    Ok(format!("\"{}\" {}", s.engine.display(), args))
}

fn installed_command() -> Option<String> {
    RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(r"System\CurrentControlSet\Services\zapret")
        .ok()?
        .get_value::<String, _>("ImagePath")
        .ok()
}

fn same_command(a: &str, b: &str) -> bool {
    let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
    norm(a) == norm(b)
}

pub struct Paused {
    strategy: String,
    running: bool,
}

pub fn pause_ours(paths: &Paths) -> Option<Paused> {
    let st = status(paths);
    let strategy = st.active_strategy()?.to_string();
    let running = st.service == ServiceState::Running;
    if running {
        run_hidden("net", &["stop", SERVICE]);
    }
    Some(Paused { strategy, running })
}

pub fn resume(paths: &Paths, p: Paused) -> anyhow::Result<String> {
    let Some(s) = strategy::discover(paths).into_iter().find(|s| s.id == p.strategy) else {
        if p.running {
            start()?;
        }
        anyhow::bail!("стратегии «{}» больше нет, служба осталась на старой версии", p.strategy);
    };
    let fresh = command(paths, &s)?;
    if installed_command().is_some_and(|old| same_command(&old, &fresh)) {
        if p.running {
            start()?;
            return Ok(format!("Служба снова запущена, стратегия «{}» не изменилась", s.name));
        }
        return Ok(format!("Стратегия службы «{}» не изменилась", s.name));
    }
    install(paths, &s)?;
    if !p.running {
        run_hidden("net", &["stop", SERVICE]);
    }
    Ok(format!("Стратегия «{}» обновилась, служба переустановлена", s.name))
}

pub fn install(paths: &Paths, s: &Strategy) -> anyhow::Result<String> {
    super::lists::ensure_user_files(&strategy::lists_dir(s, paths))?;
    remove_quiet();
    stop_engines();
    let command = command(paths, s)?;
    remove_quiet();

    let bin_path = command.replace('"', "\\\"");
    let raw = format!("create {SERVICE} binPath= \"{bin_path}\" DisplayName= \"zapret\" start= auto");
    let out = Command::new("sc").raw_arg(raw).creation_flags(CREATE_NO_WINDOW).output()?;
    if !out.status.success() {
        anyhow::bail!("sc create: {}", String::from_utf8_lossy(&out.stdout));
    }
    run_hidden("sc", &["description", SERVICE, "Zapret DPI bypass software"]);
    let (ok, o) = run_hidden("sc", &["start", SERVICE]);
    if !ok {
        anyhow::bail!("служба создана, но не запустилась: {o}");
    }
    let (_, _) = run_hidden(
        "reg",
        &[
            "add", r"HKLM\System\CurrentControlSet\Services\zapret", "/v", "zapret-discord-youtube",
            "/t", "REG_SZ", "/d", &s.id, "/f",
        ],
    );
    super::settings::set_last_strategy(&s.id);
    Ok(format!("Служба установлена со стратегией «{}»", s.name))
}

pub fn start() -> anyhow::Result<()> {
    let (ok, out) = run_hidden("sc", &["start", SERVICE]);
    if !ok {
        anyhow::bail!("{}", out.trim());
    }
    Ok(())
}

pub fn unload_windivert() -> anyhow::Result<()> {
    if process_running("HolyZapret.exe") || process_running("winws.exe") {
        anyhow::bail!("драйвер занят движком, сначала выключите защиту");
    }
    for d in ["WinDivert", "WinDivert14"] {
        run_hidden("sc", &["stop", d]);
        run_hidden("sc", &["delete", d]);
    }
    if query("WinDivert") == ServiceState::Running {
        anyhow::bail!("драйвер не выгрузился, поможет перезагрузка");
    }
    Ok(())
}

pub fn restart() -> anyhow::Result<()> {
    run_hidden("net", &["stop", SERVICE]);
    start()
}

fn remove_quiet() {
    run_hidden("net", &["stop", SERVICE]);
    run_hidden("sc", &["delete", SERVICE]);
}

pub fn remove() -> String {
    let existed = query(SERVICE) != ServiceState::NotInstalled;
    remove_quiet();
    for d in ["WinDivert", "WinDivert14"] {
        run_hidden("net", &["stop", d]);
        run_hidden("sc", &["delete", d]);
    }
    kill_process("HolyZapret.exe");
    kill_process("winws.exe");
    if existed { "Служба HolyZapret удалена".into() } else { "Служба не была установлена, процессы остановлены".into() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_compare_ignores_spacing_and_case() {
        let a = r#""C:\HZ\bin\HolyZapret.exe"  --wf-tcp=80,443 --hostlist="C:\HZ\lists\a.txt""#;
        let b = r#""c:\hz\bin\HolyZapret.exe" --wf-tcp=80,443 --hostlist="C:\HZ\lists\a.txt" "#;
        assert!(same_command(a, b));
        assert!(!same_command(a, &a.replace("443", "443,2053")));
    }
}
