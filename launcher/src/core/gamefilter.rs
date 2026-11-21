use super::paths::Paths;
use std::fs;

pub const DEFAULT_RANGE: &str = "1024-65535";
const OFF_PORT: &str = "12";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Disabled,
    All,
    Tcp,
    Udp,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameFilter {
    pub mode: Mode,
    pub tcp: String,
    pub udp: String,
}

impl Default for GameFilter {
    fn default() -> Self {
        Self { mode: Mode::Disabled, tcp: DEFAULT_RANGE.into(), udp: DEFAULT_RANGE.into() }
    }
}

impl GameFilter {
    pub fn all() -> Self {
        Self { mode: Mode::All, ..Default::default() }
    }

    pub fn enabled(&self) -> bool {
        self.mode != Mode::Disabled
    }

    pub fn tcp_value(&self) -> &str {
        if matches!(self.mode, Mode::All | Mode::Tcp) { &self.tcp } else { OFF_PORT }
    }

    pub fn udp_value(&self) -> &str {
        if matches!(self.mode, Mode::All | Mode::Udp) { &self.udp } else { OFF_PORT }
    }

    pub fn summary(&self) -> String {
        match self.mode {
            Mode::Disabled => "Выключен".into(),
            Mode::All if self.tcp == self.udp => format!("TCP и UDP {}", self.tcp),
            Mode::All => format!("TCP {} · UDP {}", self.tcp, self.udp),
            Mode::Tcp => format!("Только TCP {}", self.tcp),
            Mode::Udp => format!("Только UDP {}", self.udp),
        }
    }

    pub fn to_text(&self) -> String {
        let mode = match self.mode {
            Mode::Disabled => "disabled",
            Mode::All => "all",
            Mode::Tcp => "tcp",
            Mode::Udp => "udp",
        };
        format!("mode={mode}\r\ntcp={}\r\nudp={}\r\n", self.tcp, self.udp)
    }
}

pub fn validate(input: &str) -> Option<String> {
    let s: String = input.chars().filter(|c| !c.is_whitespace()).collect();
    if s.is_empty() {
        return None;
    }
    for item in s.split(',') {
        let (a, b) = item.split_once('-').unwrap_or((item, item));
        let ok_num = |x: &str| !x.is_empty() && !x.starts_with('0') && x.len() <= 5 && x.chars().all(|c| c.is_ascii_digit());
        if !ok_num(a) || !ok_num(b) {
            return None;
        }
        let (a, b): (u32, u32) = (a.parse().ok()?, b.parse().ok()?);
        if a > 65535 || b > 65535 || a > b {
            return None;
        }
    }
    Some(s)
}

pub fn parse(text: &str) -> GameFilter {
    let mut gf = GameFilter::all();
    let mut mode_seen = false;
    for line in text.lines() {
        let l = line.trim();
        let (k, v) = l.split_once('=').map(|(k, v)| (k.trim(), Some(v.trim()))).unwrap_or((l, None));
        match (k.to_lowercase().as_str(), v) {
            ("mode", Some(m)) => {
                mode_seen = true;
                gf.mode = match m.to_lowercase().as_str() {
                    "all" => Mode::All,
                    "tcp" => Mode::Tcp,
                    "udp" => Mode::Udp,
                    _ => Mode::Disabled,
                };
            }
            ("all", None) => {
                mode_seen = true;
                gf.mode = Mode::All;
            }
            ("tcp", Some(r)) if !r.is_empty() => {
                if let Some(r) = validate(r) {
                    gf.tcp = r;
                }
            }
            ("udp", Some(r)) if !r.is_empty() => {
                if let Some(r) = validate(r) {
                    gf.udp = r;
                }
            }
            ("tcp", _) if !mode_seen => gf.mode = Mode::Tcp,
            ("udp", _) if !mode_seen => gf.mode = Mode::Udp,
            _ => {}
        }
    }
    gf
}

pub fn load(paths: &Paths) -> GameFilter {
    match fs::read_to_string(paths.game_flag()) {
        Ok(t) => parse(&t),
        Err(_) => GameFilter::default(),
    }
}

pub fn save(paths: &Paths, gf: &GameFilter) -> anyhow::Result<()> {
    fs::write(paths.game_flag(), gf.to_text())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_validated_like_flowseal() {
        assert_eq!(validate("1024-65535").as_deref(), Some("1024-65535"));
        assert_eq!(validate(" 1024-1934, 1936-65535 ").as_deref(), Some("1024-1934,1936-65535"));
        assert_eq!(validate("443").as_deref(), Some("443"));
        assert!(validate("").is_none());
        assert!(validate("0-100").is_none());
        assert!(validate("100-50").is_none());
        assert!(validate("70000").is_none());
        assert!(validate("1-2-3").is_none());
        assert!(validate("abc").is_none());
    }

    #[test]
    fn files_parsed() {
        assert_eq!(parse("ENABLED\r\n"), GameFilter::all());
        let gf = parse("mode=tcp\r\ntcp=2000-3000\r\nudp=bad\r\n");
        assert_eq!(gf.mode, Mode::Tcp);
        assert_eq!(gf.tcp, "2000-3000");
        assert_eq!(gf.udp, DEFAULT_RANGE);
        assert_eq!(gf.tcp_value(), "2000-3000");
        assert_eq!(gf.udp_value(), "12");
        assert_eq!(parse("mode=disabled\r\n").mode, Mode::Disabled);
        assert_eq!(parse("udp\r\n").mode, Mode::Udp);
        let round = GameFilter { mode: Mode::All, tcp: "1-2".into(), udp: "3-4".into() };
        assert_eq!(parse(&round.to_text()), round);
    }
}
