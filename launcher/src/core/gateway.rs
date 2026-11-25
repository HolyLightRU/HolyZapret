use super::run_hidden;
use std::sync::Mutex;

static LAST: Mutex<Option<Iface>> = Mutex::new(None);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Iface {
    pub index: u32,
    pub alias: String,
    pub ip: String,
    pub prefix: u8,
    pub router: String,
    pub forwarding: bool,
}

const SCRIPT: &str = "$r = Get-NetRoute -DestinationPrefix '0.0.0.0/0' -ErrorAction Stop | Sort-Object RouteMetric, InterfaceMetric | Select-Object -First 1; \
$a = Get-NetIPAddress -InterfaceIndex $r.InterfaceIndex -AddressFamily IPv4 | Where-Object { $_.PrefixOrigin -ne 'WellKnown' } | Select-Object -First 1; \
$i = Get-NetIPInterface -InterfaceIndex $r.InterfaceIndex -AddressFamily IPv4; \
'' + $r.InterfaceIndex + '|' + $a.IPAddress + '|' + $a.PrefixLength + '|' + $r.NextHop + '|' + $i.Forwarding + '|' + $i.InterfaceAlias";

pub fn parse(out: &str) -> Option<Iface> {
    let line = out.lines().map(str::trim).rfind(|l| l.contains('|'))?;
    let f: Vec<&str> = line.splitn(6, '|').collect();
    if f.len() != 6 || f[1].is_empty() {
        return None;
    }
    Some(Iface {
        index: f[0].parse().ok()?,
        ip: f[1].to_string(),
        prefix: f[2].parse().ok()?,
        router: f[3].to_string(),
        forwarding: f[4].eq_ignore_ascii_case("Enabled"),
        alias: f[5].to_string(),
    })
}

pub fn iface() -> Option<Iface> {
    let full = format!("[Console]::OutputEncoding = [Text.Encoding]::UTF8; {SCRIPT}");
    let (ok, out) = run_hidden("powershell", &["-NoProfile", "-NonInteractive", "-Command", &full]);
    let i = if ok { parse(&out) } else { None };
    if let Some(i) = &i {
        *LAST.lock().unwrap() = Some(i.clone());
    }
    i
}

pub fn page_info(fallback_ip: &str) -> super::lanproxy::page::Gw {
    let last = LAST.lock().unwrap().clone();
    let (ip, prefix) = match last {
        Some(i) => (i.ip, i.prefix),
        None => (fallback_ip.to_string(), 24),
    };
    super::lanproxy::page::Gw { on: super::settings::ray_gateway(), ip, mask: mask(prefix), prefix }
}

pub fn mask(prefix: u8) -> String {
    let bits: u32 = if prefix == 0 { 0 } else { u32::MAX << (32 - prefix.min(32) as u32) };
    std::net::Ipv4Addr::from(bits).to_string()
}

fn set(index: u32, on: bool) -> anyhow::Result<()> {
    let state = if on { "forwarding=enabled" } else { "forwarding=disabled" };
    let (ok, out) = run_hidden("netsh", &["interface", "ipv4", "set", "interface", &index.to_string(), state]);
    if ok {
        Ok(())
    } else {
        anyhow::bail!("{}", out.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("netsh не смог изменить пересылку"))
    }
}

pub fn apply(on: bool) -> anyhow::Result<String> {
    let i = iface().ok_or_else(|| anyhow::anyhow!("не найдена сетевая карта с выходом в интернет"))?;
    if i.forwarding != on {
        set(i.index, on)?;
    }
    Ok(if on {
        format!("Компьютер как шлюз: включён на «{}». Шлюз для устройств: {}", i.alias, i.ip)
    } else {
        format!("Компьютер как шлюз выключен, пересылка на «{}» выключена", i.alias)
    })
}

pub fn report_line(cached: Option<&Iface>) -> String {
    let on = super::settings::ray_gateway();
    match cached {
        Some(i) => format!(
            "Компьютер как шлюз: {}; сетевая карта «{}», адрес {}/{}, роутер {}, пересылка {}",
            if on { "включён" } else { "выключен" },
            i.alias,
            i.ip,
            i.prefix,
            i.router,
            if i.forwarding { "включена" } else { "выключена" }
        ),
        None => format!("Компьютер как шлюз: {}; сетевая карта не найдена", if on { "включён" } else { "выключен" }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_interface_line() {
        let i = parse("\r\n12|192.168.100.6|24|192.168.100.1|Disabled|Ethernet 2\r\n").unwrap();
        assert_eq!(i, Iface { index: 12, alias: "Ethernet 2".into(), ip: "192.168.100.6".into(), prefix: 24, router: "192.168.100.1".into(), forwarding: false });
        assert!(parse("12||24|x|Enabled|E").is_none());
        assert!(parse("ошибка").is_none());
    }

    #[test]
    fn masks_from_prefix() {
        assert_eq!(mask(24), "255.255.255.0");
        assert_eq!(mask(16), "255.255.0.0");
        assert_eq!(mask(23), "255.255.254.0");
        assert_eq!(mask(32), "255.255.255.255");
        assert_eq!(mask(0), "0.0.0.0");
    }

    #[test]
    #[ignore]
    fn live_interface() {
        println!("{:?}", iface());
    }
}
