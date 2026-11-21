use super::run_hidden;
use std::path::PathBuf;

pub const DOWNLOAD_URL: &str = "https://one.one.one.one/";
pub const PROXY_PORT: u16 = 40000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Warp,
    Doh,
    Proxy,
    Other,
}

impl Mode {
    pub fn arg(self) -> &'static str {
        match self {
            Mode::Warp | Mode::Other => "warp",
            Mode::Doh => "doh",
            Mode::Proxy => "proxy",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Mode::Warp => "весь трафик",
            Mode::Doh => "только DNS 1.1.1.1",
            Mode::Proxy => "прокси 127.0.0.1:40000",
            Mode::Other => "особый режим",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Conn {
    Connected,
    Connecting,
    Disconnected,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct State {
    pub installed: bool,
    pub conn: Conn,
    pub mode: Mode,
    pub reason: String,
    pub needs_setup: bool,
}

impl State {
    pub fn missing() -> State {
        State { installed: false, conn: Conn::Unknown, mode: Mode::Other, reason: String::new(), needs_setup: false }
    }
}

pub fn cli() -> Option<PathBuf> {
    let base = std::env::var_os("ProgramFiles").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Program Files"));
    let p = base.join("Cloudflare").join("Cloudflare WARP").join("warp-cli.exe");
    p.is_file().then_some(p)
}

fn run(args: &[&str]) -> Option<(bool, String)> {
    let exe = cli()?;
    Some(run_hidden(&exe.display().to_string(), args))
}

fn needs_tos(out: &str) -> bool {
    let l = out.to_lowercase();
    l.contains("terms of service") || l.contains("accept-tos") || l.contains("registration missing") || l.contains("no registration")
}

pub fn parse_status(json: &str) -> (Conn, String) {
    let v: serde_json::Value = serde_json::from_str(json).unwrap_or_default();
    let status = v["status"].as_str().unwrap_or("").to_lowercase();
    let conn = match status.as_str() {
        "connected" => Conn::Connected,
        "connecting" | "reconnecting" => Conn::Connecting,
        "disconnected" => Conn::Disconnected,
        _ => Conn::Unknown,
    };
    (conn, v["reason"].as_str().unwrap_or("").to_string())
}

pub fn parse_mode(json: &str) -> Mode {
    let v: serde_json::Value = serde_json::from_str(json).unwrap_or_default();
    let m = v["settings"]["operation_mode"].as_str().unwrap_or("").to_lowercase();
    if m.contains("proxy") {
        Mode::Proxy
    } else if m.starts_with("warp") || m == "tunnel_only" {
        Mode::Warp
    } else if m == "doh" || m == "dot" {
        Mode::Doh
    } else {
        Mode::Other
    }
}

pub fn query() -> State {
    if cli().is_none() {
        return State::missing();
    }
    let (ok, status) = run(&["-j", "status"]).unwrap_or((false, String::new()));
    let (_, settings) = run(&["-j", "settings"]).unwrap_or((false, String::new()));
    let (conn, reason) = if ok { parse_status(&status) } else { (Conn::Unknown, String::new()) };
    State { installed: true, conn, mode: parse_mode(&settings), reason, needs_setup: needs_tos(&status) }
}

fn act(args: &[&str]) -> anyhow::Result<()> {
    let (ok, out) = run(args).ok_or_else(|| anyhow::anyhow!("WARP не установлен"))?;
    if needs_tos(&out) {
        anyhow::bail!("откройте приложение Cloudflare WARP и примите его условия, затем повторите");
    }
    if !ok {
        anyhow::bail!("{}", out.trim());
    }
    Ok(())
}

pub fn connect() -> anyhow::Result<()> {
    act(&["connect"])
}

pub fn disconnect() -> anyhow::Result<()> {
    act(&["disconnect"])
}

pub fn open_app() {
    if let Some(dir) = cli().and_then(|p| p.parent().map(|d| d.to_path_buf())) {
        let _ = std::process::Command::new(dir.join("Cloudflare WARP.exe")).spawn();
    }
}

pub fn set_mode(m: Mode) -> anyhow::Result<()> {
    act(&["mode", m.arg()])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_and_mode_parsed() {
        assert_eq!(parse_status(r#"{"status":"Connected"}"#).0, Conn::Connected);
        assert_eq!(parse_status("{\n  \"status\": \"Disconnected\",\n  \"reason\": \"Manual\"\n}"), (Conn::Disconnected, "Manual".into()));
        assert_eq!(parse_status("garbage").0, Conn::Unknown);
        assert_eq!(parse_mode(r#"{"settings":{"operation_mode":"warp"}}"#), Mode::Warp);
        assert_eq!(parse_mode(r#"{"settings":{"operation_mode":"proxy"}}"#), Mode::Proxy);
        assert_eq!(parse_mode(r#"{"settings":{"operation_mode":"doh"}}"#), Mode::Doh);
        assert_eq!(parse_mode(r#"{"settings":{"operation_mode":"WarpProxy"}}"#), Mode::Proxy);
        assert_eq!(parse_mode(r#"{"settings":{"operation_mode":"warp+doh"}}"#), Mode::Warp);
        assert!(needs_tos("Error: You must accept the Terms of Service (--accept-tos)"));
        assert!(!needs_tos("Success"));
    }
}


