use super::control::{self, NetProfile};
use crate::core::run_hidden;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Ok,
    Info,
    Warn,
    Bad,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fix {
    MakePrivate(u32),
    RemoveBlocks(Vec<String>),
    AddRule,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    pub level: Level,
    pub title: String,
    pub text: String,
    pub fix: Option<Fix>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub block_all: Vec<String>,
    pub blocks: Vec<String>,
    pub ours: usize,
    pub av: Vec<String>,
    pub fw: Vec<String>,
    pub vpn: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Input {
    pub addr: Option<String>,
    pub local_ok: Option<bool>,
    pub profile: Option<NetProfile>,
    pub report: Option<Report>,
}

const SCRIPT: &str = r#"
$ErrorActionPreference = 'SilentlyContinue'
$ProgressPreference = 'SilentlyContinue'
[Console]::OutputEncoding = [Text.Encoding]::UTF8
$exe = '__EXE__'
$apps = Get-NetFirewallApplicationFilter | Where-Object { $_.Program -and ($_.Program -ieq $exe -or $_.Program -match 'holyzapret') }
$rules = @()
foreach ($a in $apps) { $rules += $a | Get-NetFirewallRule }
$r = [ordered]@{
  block_all = @(Get-NetFirewallProfile | Where-Object { $_.Enabled -eq 'True' -and $_.AllowInboundRules -eq 'False' } | ForEach-Object { [string]$_.Name })
  blocks = @($rules | Where-Object { $_.Action -eq 'Block' -and $_.Enabled -eq 'True' -and $_.Direction -eq 'Inbound' } | ForEach-Object { [string]$_.Name } | Select-Object -Unique)
  ours = @(Get-NetFirewallRule -DisplayName 'HolyZapret LAN proxy' | Where-Object { $_.Enabled -eq 'True' }).Count
  av = @(Get-CimInstance -Namespace root/SecurityCenter2 -ClassName AntiVirusProduct | ForEach-Object { [string]$_.displayName })
  fw = @(Get-CimInstance -Namespace root/SecurityCenter2 -ClassName FirewallProduct | ForEach-Object { [string]$_.displayName })
  vpn = @(Get-NetAdapter | Where-Object { $_.Status -eq 'Up' -and $_.InterfaceDescription -match 'TAP|TUN|Wintun|WireGuard|VPN|Radmin|ZeroTier|Hamachi|Outline|Tailscale|Cloudflare' } | ForEach-Object { [string]$_.Name })
}
$r | ConvertTo-Json -Depth 3 -Compress
"#;

fn encoded(script: &str) -> String {
    let utf16: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    STANDARD.encode(utf16)
}

fn powershell(script: &str) -> (bool, String) {
    run_hidden("powershell", &["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-EncodedCommand", &encoded(script)])
}

pub fn parse_report(json: &str) -> Option<Report> {
    let line = json.lines().map(str::trim).find(|l| l.starts_with('{'))?;
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    let list = |k: &str| -> Vec<String> {
        match &v[k] {
            serde_json::Value::Array(a) => a.iter().filter_map(|x| x.as_str()).map(String::from).filter(|s| !s.is_empty()).collect(),
            serde_json::Value::String(s) if !s.is_empty() => vec![s.clone()],
            _ => Vec::new(),
        }
    };
    Some(Report {
        block_all: list("block_all"),
        blocks: list("blocks"),
        ours: v["ours"].as_u64().unwrap_or(0) as usize,
        av: list("av"),
        fw: list("fw"),
        vpn: list("vpn"),
    })
}

fn query() -> Option<Report> {
    let exe = std::env::current_exe().ok()?.display().to_string().replace('\'', "''");
    let (_, out) = powershell(&SCRIPT.replace("__EXE__", &exe));
    parse_report(&out)
}

fn local_check(addr: &str) -> bool {
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(5))).build().into();
    agent.get(&format!("http://{addr}/proxy.pac")).call().is_ok_and(|r| r.status().as_u16() == 200)
}

fn profile_name(p: &str) -> &str {
    match p.to_lowercase().as_str() {
        "public" => "общественной",
        "private" => "частной",
        "domain" => "доменной",
        _ => "текущей",
    }
}

fn third_party(names: &[String], ours: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for n in names {
        let l = n.to_lowercase();
        if !ours.iter().any(|o| l.contains(o)) && !out.iter().any(|x| x.eq_ignore_ascii_case(n)) {
            out.push(n.clone());
        }
    }
    out
}

pub fn findings(i: &Input) -> Vec<Finding> {
    let mut out = Vec::new();
    let mut add = |level: Level, title: String, text: String, fix: Option<Fix>| out.push(Finding { level, title, text, fix });
    let addr = i.addr.clone().unwrap_or_default();
    match i.local_ok {
        Some(true) => add(Level::Ok, "Раздача работает на компьютере".into(), format!("Страница настройки открывается по адресу http://{addr}/"), None),
        Some(false) => add(
            Level::Bad,
            "Раздача не отвечает даже на самом компьютере".into(),
            "Выключите и снова включите раздачу. Если не помогло, смените порт: его могла занять другая программа.".into(),
            None,
        ),
        None => add(Level::Bad, "Раздача выключена".into(), "Включите раздачу и запустите проверку ещё раз.".into(), None),
    }
    if let Some(p) = &i.profile {
        if p.public {
            let name = if p.name.is_empty() { String::new() } else { format!(" «{}»", p.name) };
            add(
                Level::Bad,
                format!("Сеть{name} помечена как общественная"),
                "В общественных сетях Windows не пускает к компьютеру другие устройства. Если это ваша домашняя сеть, сделайте её частной.".into(),
                Some(Fix::MakePrivate(p.index)),
            );
        } else {
            add(Level::Ok, "Сеть помечена как частная".into(), "Windows разрешает устройствам в этой сети подключаться к компьютеру.".into(), None);
        }
    }
    match &i.report {
        None => add(Level::Info, "Брандмауэр проверить не удалось".into(), "Windows не ответила на запрос о правилах брандмауэра.".into(), None),
        Some(r) => {
            if !r.block_all.is_empty() {
                let where_ = r.block_all.iter().map(|p| profile_name(p)).collect::<Vec<_>>().join(", ");
                add(
                    Level::Bad,
                    "В брандмауэре включено «Блокировать все входящие подключения»".into(),
                    format!("Так сделано для {where_} сети. Выключите эту галочку: Безопасность Windows → Брандмауэр и защита сети → ваша сеть."),
                    None,
                );
            }
            if !r.blocks.is_empty() {
                add(
                    Level::Bad,
                    "Брандмауэр запрещает лаунчеру входящие подключения".into(),
                    format!("Такие правила Windows создаёт, если в её окне про брандмауэр нажать «Отмена». Найдено правил: {}.", r.blocks.len()),
                    Some(Fix::RemoveBlocks(r.blocks.clone())),
                );
            }
            if r.ours == 0 {
                add(
                    Level::Warn,
                    "Нет разрешающего правила HolyZapret в брандмауэре".into(),
                    "Без него телефон может не достучаться до компьютера. Лаунчер добавит правило заново.".into(),
                    Some(Fix::AddRule),
                );
            }
            if r.block_all.is_empty() && r.blocks.is_empty() && r.ours > 0 {
                add(Level::Ok, "Брандмауэр Windows пропускает раздачу".into(), "Разрешающее правило есть, запрещающих нет.".into(), None);
            }
            let foreign: Vec<String> = third_party(&r.av, &["defender", "защитник"])
                .into_iter()
                .chain(third_party(&r.fw, &["windows", "брандмауэр", "defender", "защитник"]))
                .fold(Vec::new(), |mut acc, n| {
                    if !acc.iter().any(|x: &String| x.eq_ignore_ascii_case(&n)) {
                        acc.push(n);
                    }
                    acc
                });
            if !foreign.is_empty() {
                add(
                    Level::Warn,
                    format!("Установлен {}", foreign.join(", ")),
                    "У него может быть свой сетевой экран. Если телефон не подключается, разрешите «HolyZapret Launcher» входящие подключения в домашней сети или добавьте его в исключения.".into(),
                    None,
                );
            }
            if !r.vpn.is_empty() {
                add(
                    Level::Info,
                    format!("Включены VPN-адаптеры: {}", r.vpn.join(", ")),
                    "Обычно они не мешают. Если телефон не видит компьютер, попробуйте на время отключить VPN.".into(),
                    None,
                );
            }
        }
    }
    let prefix = addr.split(':').next().and_then(|ip| ip.rsplit_once('.').map(|(p, _)| format!("{p}."))).unwrap_or_default();
    let phone = if prefix.is_empty() { String::new() } else { format!(" Адрес телефона в настройках Wi-Fi должен быть вида {prefix}x") };
    add(
        Level::Info,
        "Если всё в порядке, а телефон не открывает страницу".into(),
        format!("Проверьте, что телефон в той же сети, что и компьютер, и не в гостевой.{phone}. Если и это так, выключите на роутере изоляцию клиентов (AP Isolation)."),
        None,
    );
    out
}

pub fn run() -> Vec<Finding> {
    let addr = control::address();
    let local_ok = addr.as_deref().map(local_check);
    let profile = control::net_profile();
    let report = query();
    findings(&Input { addr, local_ok, profile, report })
}

pub fn apply(fix: &Fix) -> anyhow::Result<String> {
    match fix {
        Fix::MakePrivate(i) => control::make_private(*i).map(|_| "Сеть стала частной".to_string()),
        Fix::RemoveBlocks(names) => {
            let list = names.iter().map(|n| format!("'{}'", n.replace('\'', "''"))).collect::<Vec<_>>().join(",");
            let (ok, out) = powershell(&format!("Remove-NetFirewallRule -Name {list} -ErrorAction Stop"));
            if ok {
                Ok("Запрещающие правила брандмауэра удалены".into())
            } else {
                anyhow::bail!("{}", out.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("Windows не дала удалить правила"))
            }
        }
        Fix::AddRule => control::repair_firewall().map(|_| "Разрешающее правило брандмауэра добавлено".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_parsed() {
        let r = parse_report("junk\r\n{\"block_all\":[\"Public\"],\"blocks\":\"{abc}\",\"ours\":2,\"av\":[\"Kaspersky Free\",\"Windows Defender\"],\"fw\":[],\"vpn\":[\"Radmin VPN\"]}").unwrap();
        assert_eq!(r.block_all, vec!["Public"]);
        assert_eq!(r.blocks, vec!["{abc}"]);
        assert_eq!(r.ours, 2);
        assert_eq!(r.av.len(), 2);
        assert!(parse_report("ошибка").is_none());
        let noisy = "{\"block_all\":[],\"blocks\":[],\"ours\":2,\"av\":[],\"fw\":[],\"vpn\":[]}
#< CLIXML
<Objs><Obj S=\"progress\"></Obj></Objs>";
        assert_eq!(parse_report(noisy).unwrap().ours, 2);
    }

    #[test]
    fn findings_explain_problems() {
        let bad = Input {
            addr: Some("192.168.1.101:8881".into()),
            local_ok: Some(true),
            profile: Some(NetProfile { public: true, index: 12, name: "Сеть".into() }),
            report: Some(Report {
                block_all: vec![],
                blocks: vec!["{a}".into(), "{b}".into()],
                ours: 0,
                av: vec!["Kaspersky Free".into(), "Windows Defender".into()],
                fw: vec!["Kaspersky Free".into()],
                vpn: vec![],
            }),
        };
        let f = findings(&bad);
        assert!(f.iter().any(|x| x.fix == Some(Fix::MakePrivate(12)) && x.level == Level::Bad));
        assert!(f.iter().any(|x| x.fix == Some(Fix::RemoveBlocks(vec!["{a}".into(), "{b}".into()]))));
        assert!(f.iter().any(|x| x.fix == Some(Fix::AddRule)));
        assert_eq!(f.iter().filter(|x| x.title.contains("Kaspersky")).count(), 1);
        assert!(!f.iter().any(|x| x.title.contains("Defender")));
        assert!(f.last().unwrap().text.contains("192.168.1."));
        let good = Input {
            addr: Some("10.0.0.5:8881".into()),
            local_ok: Some(true),
            profile: Some(NetProfile { public: false, index: 3, name: String::new() }),
            report: Some(Report { ours: 2, av: vec!["Windows Defender".into()], ..Default::default() }),
        };
        let f = findings(&good);
        assert!(f.iter().all(|x| matches!(x.level, Level::Ok | Level::Info)), "{f:?}");
        assert!(f.iter().all(|x| x.fix.is_none()));
    }

    #[test]
    fn encoded_command_is_utf16() {
        assert_eq!(encoded("ab"), "YQBiAA==");
    }
}
