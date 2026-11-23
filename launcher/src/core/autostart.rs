use super::run_hidden;
use std::path::Path;

pub const TASK: &str = "HolyZapret Launcher";
pub const TRAY_ARG: &str = "--tray";

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

pub fn task_xml(exe: &Path, user: &str) -> String {
    let exe = xml_escape(&exe.display().to_string());
    let user = xml_escape(user);
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>Запуск лаунчера HolyZapret в трее при входе в Windows</Description>
  </RegistrationInfo>
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
      <UserId>{user}</UserId>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>{user}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>HighestAvailable</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>7</Priority>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{exe}</Command>
      <Arguments>{TRAY_ARG}</Arguments>
    </Exec>
  </Actions>
</Task>
"#
    )
}

fn user() -> String {
    let name = std::env::var("USERNAME").unwrap_or_default();
    match std::env::var("USERDOMAIN") {
        Ok(d) if !d.is_empty() => format!("{d}\\{name}"),
        _ => name,
    }
}

pub fn enabled() -> bool {
    run_hidden("schtasks", &["/Query", "/TN", TASK]).0
}

pub fn set(on: bool) -> anyhow::Result<()> {
    set_exe(on, &std::env::current_exe()?)
}

pub fn set_exe(on: bool, exe: &Path) -> anyhow::Result<()> {
    if !on {
        let (ok, out) = run_hidden("schtasks", &["/Delete", "/TN", TASK, "/F"]);
        if !ok && enabled() {
            anyhow::bail!("{}", out.trim());
        }
        return Ok(());
    }
    let xml = task_xml(exe, &user());
    let path = std::env::temp_dir().join("holyzapret-autostart.xml");
    let mut bytes = vec![0xFF, 0xFE];
    for u in xml.encode_utf16() {
        bytes.extend_from_slice(&u.to_le_bytes());
    }
    std::fs::write(&path, bytes)?;
    let p = path.display().to_string();
    let (ok, out) = run_hidden("schtasks", &["/Create", "/TN", TASK, "/XML", &p, "/F"]);
    let _ = std::fs::remove_file(&path);
    if !ok {
        anyhow::bail!("{}", out.trim());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_has_safe_settings() {
        let x = task_xml(Path::new(r"C:\A & B\HolyZapret Launcher.exe"), r"PC\user");
        assert!(x.contains(r"<Command>C:\A &amp; B\HolyZapret Launcher.exe</Command>"));
        assert!(x.contains("<Arguments>--tray</Arguments>"));
        assert!(x.contains("<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>"));
        assert!(x.contains("<DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>"));
        assert!(x.contains("<RunLevel>HighestAvailable</RunLevel>"));
        assert!(x.contains(r"<UserId>PC\user</UserId>"));
    }
}

