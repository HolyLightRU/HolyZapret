use crate::desync::{self, Config, Technique};
use crate::lists::{Hosts, Nets};
use crate::packet::{self, FIN, RST, SYN};
use crate::procs::{AppRule, Apps, Conn, Owner, Source};
use crate::profiles::{self, Own, Ports};
use crate::tls;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, Instant};

mod freeze;
mod quicflow;
mod store;
mod udpauto;

pub use freeze::{FETCH_WANT, FREEZE_HI, FREEZE_LO, FREEZE_STALL, Fetch, fetch_verdict};
pub use quicflow::{QuicCfg, QuicMode};
pub use udpauto::{UDP_KEY, UDP_ROTATION, UDP_SWITCH_AFTER, UDP_WHY, UDP_WINDOW, udp_key};

pub const MAX_REASM: usize = 16384;
pub const REASM_WAIT: Duration = Duration::from_millis(300);
pub const IDLE: Duration = Duration::from_secs(60);
pub const MAX_FLOWS: usize = 4096;
pub const SILENT: Duration = Duration::from_secs(4);
pub const MAX_MEMORY: usize = 4096;
const FAIL_LOG: usize = 32;
const FAIL_KEEP: Duration = Duration::from_secs(31 * 60);
pub const BG_APP: &str = "проверка самоподстройки";
pub const BG_IDLE: Duration = Duration::from_secs(30);
pub const BG_EVERY: Duration = Duration::from_secs(12);
const BG_AT_ONCE: usize = 2;
pub const AUTO_FAILS: u32 = 2;
pub const AUTO_WINDOW: Duration = Duration::from_secs(120);
pub const PROBE: &str = "probe";

type Key = (IpAddr, u16, IpAddr, u16);

enum Stage<A> {
    Fresh,
    Reasm { need: usize, buf: Vec<u8>, held: Vec<(Vec<u8>, A)>, next_seq: u32, since: Instant },
    Done,
}

struct Flow<A> {
    isn: Option<u32>,
    stage: Stage<A>,
    last: Instant,
    hops: Option<u8>,
    srv_isn: Option<u32>,
    watch: Option<freeze::Watch>,
    sent: Option<(u32, &'static str)>,
    redo: Option<&'static str>,
    resent: u8,
    wscale: Option<u8>,
    wss: u8,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Action<A> {
    Pass,
    Hold,
    Drop,
    Send(Vec<(Vec<u8>, A)>),
    Reject(Vec<u8>, A),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Answered,
    Reset,
    Silent,
    Frozen,
}

impl Outcome {
    pub fn name(self) -> &'static str {
        match self {
            Outcome::Answered => "answered",
            Outcome::Reset => "reset",
            Outcome::Silent => "silent",
            Outcome::Frozen => "frozen",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TechStats {
    pub used: u64,
    pub answered: u64,
    pub reset: u64,
    pub silent: u64,
    pub frozen: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub handled: u64,
    pub skipped: u64,
    pub no_ts: u64,
    pub fallback: u64,
    pub voice: u64,
    pub game_udp: u64,
    pub auto_added: u64,
    pub flushed: u64,
    pub resent: u64,
    pub app_skipped: u64,
    pub app_all: u64,
    pub answered: u64,
    pub reset: u64,
    pub silent: u64,
    pub frozen: u64,
    pub rst_filter: u64,
    pub rst_server: u64,
    pub rst_far: u64,
    pub rst_blind: u64,
    pub by_tech: BTreeMap<&'static str, TechStats>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RstFrom {
    Filter,
    Server,
    Far,
    Blind,
}

pub fn reset_from(rst_hops: u8, server_hops: Option<u8>) -> RstFrom {
    let Some(server) = server_hops else { return RstFrom::Blind };
    match server as i16 - rst_hops as i16 {
        d if d >= 2 => RstFrom::Filter,
        d if d <= -2 => RstFrom::Far,
        _ => RstFrom::Server,
    }
}

const RST_NOTES: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mode {
    Fixed(Technique),
    Adaptive(Vec<Technique>),
}

impl Mode {
    pub fn parse(name: &str) -> Option<Mode> {
        let (head, start) = match name.split_once(':') {
            Some((h, s)) => (h, Some(s)),
            None => (name, None),
        };
        if head.eq_ignore_ascii_case("adaptive") {
            let mut list = vec![
                desync::GENERAL,
                desync::SNI_SPLIT,
                desync::POOLED,
                desync::NONE,
                desync::TTL,
                desync::DISORDER,
                desync::MD5SIG,
                desync::BADSEQ,
                desync::OVERLAP_MAX,
                desync::RANDOM,
            ];
            if let Some(s) = start.filter(|s| !s.is_empty()) {
                let first = desync::technique(s)?;
                list.retain(|t| t.name != first.name);
                list.insert(0, first);
            }
            return Some(Mode::Adaptive(list));
        }
        if start.is_some() {
            return None;
        }
        desync::technique(name).map(Mode::Fixed)
    }

    pub fn is_adaptive(name: &str) -> bool {
        name.split(':').next().is_some_and(|h| h.eq_ignore_ascii_case("adaptive"))
    }

    pub fn name(&self) -> &'static str {
        match self {
            Mode::Fixed(t) => t.name,
            Mode::Adaptive(_) => "adaptive",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Memory {
    pub idx: usize,
    pub ok: u32,
    pub fails: u32,
    pub streak: u32,
    pub hint: bool,
}

pub const SWITCH_AFTER: u32 = 2;
pub const NODE: &str = "node:";
pub const STAT: &str = "stat:";
pub const NAME: &str = "name:";
pub const SITE_NAME: &str = "fakename:";
pub const QUIET_APPS: [&str; 2] = ["holyzapret launcher.exe", "holyzapret-launcher.exe"];
const SAME_SKIP: usize = 40;
pub const NODE_TRUST: u32 = 2;

pub fn node_key(ip: &IpAddr) -> String {
    match ip {
        IpAddr::V4(a) => {
            let o = a.octets();
            format!("{NODE}{}.{}.{}", o[0], o[1], o[2])
        }
        IpAddr::V6(a) => {
            let s = a.segments();
            format!("{NODE}{:x}:{:x}:{:x}", s[0], s[1], s[2])
        }
    }
}
const RESEND_MAX: u8 = 3;

pub fn site_base(host: &str) -> Option<&str> {
    let labels: Vec<&str> = host.split('.').collect();
    let n = labels.len();
    if n < 2 || labels.iter().any(|l| l.is_empty()) {
        return None;
    }
    let keep = if n >= 3 && labels[n - 1].len() == 2 && labels[n - 2].len() <= 3 { 3 } else { 2 };
    let len: usize = labels[n - keep..].iter().map(|l| l.len() + 1).sum::<usize>() - 1;
    Some(&host[host.len() - len..])
}

struct Pending {
    host: Option<String>,
    tech: &'static str,
    since: Instant,
    id: u64,
    ack0: Option<u32>,
    dcid: Option<Vec<u8>>,
    bg: bool,
    node: Option<IpAddr>,
    fname: Option<&'static str>,
}

struct Target {
    addr: SocketAddr,
    seen: Instant,
    parts: u8,
    probed: Option<Instant>,
}

#[derive(Default)]
pub struct Lists {
    pub hosts: Hosts,
    pub host_excludes: Hosts,
    pub ip_excludes: Nets,
    pub apps: Apps,
    pub google: Hosts,
    pub auto: Hosts,
    pub ipset: Nets,
    pub own: Own,
    pub game_tcp: Ports,
    pub game_udp: Ports,
}

pub const RECENT: usize = 300;
pub const DEVICE: &str = "устройство ";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decision {
    pub id: u64,
    pub technique: Option<&'static str>,
    pub outcome: Option<Outcome>,
    pub time: String,
    pub host: Option<String>,
    pub app: Option<String>,
    pub pid: Option<u32>,
    pub handled: bool,
    pub why: &'static str,
    pub parts: u8,
}

pub type AppLookup = Box<dyn FnMut(&Conn, Instant) -> Option<Owner> + Send>;
pub type UdpLookup = Box<dyn FnMut(u16, Instant) -> Option<Owner> + Send>;

pub struct Engine<A> {
    cfg: Config,
    lists: Lists,
    flows: HashMap<Key, Flow<A>>,
    rnd: Box<dyn FnMut(&mut [u8]) + Send>,
    app_of: Option<AppLookup>,
    mode: Mode,
    pending: HashMap<Key, Pending>,
    memory: store::Store,
    next_id: u64,
    pub verbose: bool,
    pub track: bool,
    pub notes: Vec<String>,
    pub recent: VecDeque<Decision>,
    clock: fn() -> String,
    pub stats: Stats,
    quic: Option<QuicCfg>,
    qflows: HashMap<Key, quicflow::QuicFlow<A>>,
    game_seen: HashMap<Key, (u32, Instant)>,
    pub autolist: bool,
    pub forward: bool,
    qmemory: store::Store,
    nodes: store::Store,
    auto_fails: HashMap<String, (u32, Instant)>,
    auto_new: Vec<String>,
    fail_log: HashMap<String, Vec<(Instant, &'static str, bool)>>,
    rst_noted: HashSet<String>,
    rank: HashMap<&'static str, (u32, u32)>,
    delayed: Vec<(Instant, Vec<(Vec<u8>, A)>)>,
    name_rank: HashMap<&'static str, (u32, u32)>,
    site_names: HashMap<String, &'static str>,
    tried_names: HashMap<String, Vec<&'static str>>,
    burned: HashMap<String, Vec<&'static str>>,
    redeemed: Vec<Instant>,
    targets: HashMap<String, Target>,
    bg_ports: HashSet<u16>,
    udp_of: Option<UdpLookup>,
    uflows: HashMap<Key, udpauto::UFlow>,
    upending: HashMap<Key, Pending>,
    umemory: udpauto::UdpMemory,
    noise: HashMap<String, udpauto::Noise>,
    ulast_ok: HashMap<String, Instant>,
    warp: Option<std::sync::Arc<crate::warp::Shared>>,
    frozen: freeze::FrozenMap,
    later: Vec<String>,
    flush_dns: bool,
    pub dns_alt: bool,
}

impl<A: Clone> Engine<A> {
    pub fn new(cfg: Config, lists: Lists, rnd: Box<dyn FnMut(&mut [u8]) + Send>) -> Self {
        Engine {
            cfg,
            lists,
            flows: HashMap::new(),
            rnd,
            app_of: None,
            mode: Mode::Fixed(desync::GENERAL),
            pending: HashMap::new(),
            memory: store::Store::new(),
            next_id: 0,
            verbose: false,
            track: false,
            notes: Vec::new(),
            recent: VecDeque::new(),
            clock: || String::new(),
            stats: Stats::default(),
            quic: None,
            qflows: HashMap::new(),
            game_seen: HashMap::new(),
            autolist: false,
            forward: false,
            qmemory: store::Store::new(),
            nodes: store::Store::with_limit(MAX_MEMORY),
            auto_fails: HashMap::new(),
            auto_new: Vec::new(),
            fail_log: HashMap::new(),
            rst_noted: HashSet::new(),
            rank: HashMap::new(),
            delayed: Vec::new(),
            name_rank: HashMap::new(),
            site_names: HashMap::new(),
            tried_names: HashMap::new(),
            burned: HashMap::new(),
            redeemed: Vec::new(),
            targets: HashMap::new(),
            bg_ports: HashSet::new(),
            udp_of: None,
            uflows: HashMap::new(),
            upending: HashMap::new(),
            umemory: HashMap::new(),
            noise: HashMap::new(),
            ulast_ok: HashMap::new(),
            warp: None,
            frozen: HashMap::new(),
            later: Vec::new(),
            flush_dns: false,
            dns_alt: false,
        }
    }

    pub fn take_redeemed(&mut self) -> Vec<Instant> {
        std::mem::take(&mut self.redeemed)
    }

    pub fn add_bg_port(&mut self, port: u16) {
        self.bg_ports.insert(port);
    }

    pub fn remove_bg_port(&mut self, port: u16) {
        self.bg_ports.remove(&port);
    }

    pub fn bg_targets(&mut self, now: Instant) -> Vec<(String, SocketAddr, u8)> {
        let Mode::Adaptive(list) = &self.mode else { return Vec::new() };
        let total = list.iter().filter(|t| t.name != desync::WARP.name).count();
        self.targets.retain(|_, t| now.saturating_duration_since(t.seen) < FAIL_KEEP);
        let busy: HashSet<&str> = self.pending.values().filter_map(|p| p.host.as_deref()).collect();
        let mut ready: Vec<(Option<Instant>, String)> = Vec::new();
        for (h, t) in self.targets.iter() {
            let Some(log) = self.fail_log.get(h).filter(|l| !l.is_empty()) else { continue };
            let mut tried: Vec<&str> = log.iter().map(|e| e.1).collect();
            tried.sort_unstable();
            tried.dedup();
            let idle = now.saturating_duration_since(t.seen) >= BG_IDLE;
            let rested = t.probed.is_none_or(|p| now.saturating_duration_since(p) >= BG_EVERY);
            if tried.len() >= total || !idle || !rested || busy.contains(h.as_str()) {
                continue;
            }
            ready.push((t.probed, h.clone()));
        }
        ready.sort();
        let mut out = Vec::new();
        for (_, h) in ready.into_iter().take(BG_AT_ONCE) {
            if let Some(t) = self.targets.get_mut(&h) {
                t.probed = Some(now);
                out.push((h, t.addr, t.parts));
            }
        }
        out
    }

    fn note_target(&mut self, host: &str, addr: SocketAddr, parts: usize, now: Instant) {
        if self.forward || (self.targets.len() >= MAX_MEMORY && !self.targets.contains_key(host)) {
            return;
        }
        let probed = self.targets.get(host).and_then(|t| t.probed);
        self.targets.insert(host.to_string(), Target { addr, seen: now, parts: parts.min(255) as u8, probed });
    }

    fn note_outcome(&mut self, host: &str, tech: &'static str, at: Instant, outcome: Outcome, user: bool) {
        if outcome == Outcome::Answered {
            if let Some(log) = self.fail_log.remove(host) {
                self.redeemed.extend(log.into_iter().filter(|e| e.1 != tech && e.2).map(|e| e.0));
            }
            return;
        }
        if self.fail_log.len() >= MAX_MEMORY && !self.fail_log.contains_key(host) {
            self.fail_log.retain(|_, log| log.last().is_some_and(|e| at.saturating_duration_since(e.0) < FAIL_KEEP));
            if self.fail_log.len() >= MAX_MEMORY {
                return;
            }
        }
        let log = self.fail_log.entry(host.to_string()).or_default();
        log.retain(|e| at.saturating_duration_since(e.0) < FAIL_KEEP);
        if log.len() >= FAIL_LOG {
            let repeated = (0..log.len()).find(|&i| log.iter().filter(|e| e.1 == log[i].1).count() > 1);
            match repeated {
                Some(i) => {
                    log.remove(i);
                }
                None => return,
            }
        }
        log.push((at, tech, user));
    }

    pub fn take_auto_new(&mut self) -> Vec<String> {
        std::mem::take(&mut self.auto_new)
    }

    fn probe_result(&mut self, p: &Pending, outcome: Outcome) {
        let Some(h) = p.host.clone() else { return };
        if outcome == Outcome::Answered {
            self.auto_fails.remove(&h);
            return;
        }
        if self.auto_fails.len() >= MAX_MEMORY {
            self.auto_fails.clear();
        }
        let e = self.auto_fails.entry(h.clone()).or_insert((0, p.since));
        if p.since.saturating_duration_since(e.1) > AUTO_WINDOW {
            *e = (0, p.since);
        }
        e.0 += 1;
        if e.0 >= AUTO_FAILS && !self.lists.auto.matches(&h) {
            self.auto_fails.remove(&h);
            self.lists.auto.add(&h);
            self.auto_new.push(h.clone());
            self.stats.auto_added += 1;
            self.note(format!("{h}: дважды без ответа, добавлен в автосписок"));
        }
    }

    pub fn with_quic(mut self, cfg: QuicCfg) -> Self {
        self.quic = Some(cfg);
        self
    }

    pub fn quic_mode(&self) -> Option<QuicMode> {
        self.quic.as_ref().map(|q| q.mode)
    }

    fn owner_of(&mut self, src: IpAddr, local_port: u16, remote: IpAddr, remote_port: u16, now: Instant) -> Option<Owner> {
        if self.forward {
            return Some(Owner { pid: 0, name: Some(format!("{DEVICE}{src}")), source: Source::Table });
        }
        match (&mut self.app_of, self.lists.apps.is_empty() && !self.verbose && !self.track) {
            (Some(lookup), false) => lookup(&Conn { local_port, remote, remote_port }, now),
            _ => None,
        }
    }

    pub fn wants_dns(&self, host: &str) -> bool {
        !self.lists.host_excludes.matches(host) && (self.lists.hosts.matches(host) || self.lists.google.matches(host) || self.lists.auto.matches(host))
    }

    fn verdict(&mut self, host: Option<&str>, dst: &IpAddr, dport: u16, udp: bool, owner: &Option<Owner>) -> (bool, &'static str, bool) {
        let app = owner.as_ref().and_then(|o| o.name.clone());
        let rule = self.lists.apps.rule(app.as_deref());
        let own = self.lists.own;
        let excluded = self.excluded(host, dst);
        let listed = host.is_some_and(|h| self.lists.hosts.matches(h)) && (dport == 443 || (!udp && dport == 80 && own.http));
        let auto = dport == 443 && host.is_some_and(|h| self.lists.auto.matches(h));
        let cf = !udp && own.cf && profiles::CF_PORTS.contains(&dport) && host.is_some_and(|h| profiles::host_is(h, profiles::CF_HOST));
        let google = !udp && own.google && dport == 443 && host.is_some_and(|h| self.lists.google.matches(h));
        let myip_port = if udp { own.myip_quic && dport == 443 } else { own.myip_tcp && profiles::MYIP_TCP_PORTS.contains(&dport) };
        let myip = myip_port && self.lists.ipset.contains(dst);
        let r = match rule {
            AppRule::Skip => (false, "программа в исключениях", false),
            _ if cf => (true, "Discord на своих портах", false),
            _ if google => (true, "сайт Google", true),
            _ if excluded => (false, "сайт в исключениях", false),
            AppRule::All if dport == 443 => (true, "вся программа", false),
            _ if listed => (true, "сайт в списке", false),
            _ if auto => (true, "сайт в автосписке", false),
            _ if myip => (true, "адрес в «Мои IP»", false),
            _ => (false, "сайта нет в списке", false),
        };
        match rule {
            AppRule::Skip => self.stats.app_skipped += 1,
            AppRule::All if r.1 == "вся программа" => self.stats.app_all += 1,
            _ => {}
        }
        r
    }

    fn who(host: &Option<String>, owner: &Option<Owner>) -> String {
        match owner {
            Some(o) => format!("{} ({}, pid {}, {})", host.as_deref().unwrap_or("без имени"), o.name.as_deref().unwrap_or("имя неизвестно"), o.pid, o.source.label()),
            None => format!("{} (программа неизвестна)", host.as_deref().unwrap_or("без имени")),
        }
    }

    pub fn with_apps(mut self, lookup: AppLookup) -> Self {
        self.app_of = Some(lookup);
        self
    }

    pub fn with_warp(mut self, shared: std::sync::Arc<crate::warp::Shared>) -> Self {
        if let Mode::Adaptive(list) = &mut self.mode {
            if !list.iter().any(|t| t.name == desync::WARP.name) {
                list.push(desync::WARP);
            }
        }
        self.warp = Some(shared);
        let later = std::mem::take(&mut self.later);
        self.load_memory(&later.join("\n"));
        self
    }

    pub fn warp_settle(&mut self, host: Option<String>, local_port: u16, dst: IpAddr, dport: u16, outcome: Outcome, now: Instant) {
        let owner = self.owner_of(dst, local_port, dst, dport, now);
        let id = self.record(&host, &owner, true, crate::warp::WHY, Some(desync::WARP.name));
        self.stats.handled += 1;
        self.stats.by_tech.entry(desync::WARP.name).or_default().used += 1;
        let p = Pending { host, tech: desync::WARP.name, since: now, id, ack0: None, dcid: None, bg: false, node: None, fname: None };
        self.settle(p, outcome);
    }

    pub fn with_udp_apps(mut self, lookup: UdpLookup) -> Self {
        self.udp_of = Some(lookup);
        self
    }

    pub fn with_mode(mut self, mode: Mode) -> Self {
        self.mode = mode;
        self
    }

    pub fn mode(&self) -> &Mode {
        &self.mode
    }

    fn choose(&mut self, host: Option<&str>, addr: Option<&IpAddr>) -> Technique {
        match &self.mode {
            Mode::Fixed(t) => *t,
            Mode::Adaptive(list) => {
                let m = host.and_then(|h| self.memory_for(h)).or_else(|| self.node_hint(addr)).unwrap_or_default();
                list[m.idx % list.len()]
            }
        }
    }

    fn node_hint(&self, addr: Option<&IpAddr>) -> Option<Memory> {
        let Mode::Adaptive(list) = &self.mode else { return None };
        let m = self.nodes.get(&node_key(addr?))?;
        let trusted = m.ok >= NODE_TRUST && m.streak == 0 && m.idx < list.len() && list[m.idx].name != desync::WARP.name;
        trusted.then(|| Memory { idx: m.idx, hint: m.idx != 0, ..Default::default() })
    }

    pub fn fake_name(&self, host: Option<&str>) -> &'static str {
        if let Some(mut h) = host {
            loop {
                if let Some(n) = self.site_names.get(h) {
                    return n;
                }
                let Some((_, rest)) = h.split_once('.') else { break };
                if !rest.contains('.') {
                    break;
                }
                h = rest;
            }
        }
        let tried = host.and_then(|h| self.tried_names.get(h));
        let score = |n: &str| {
            let (ok, fails) = self.name_rank.get(n).copied().unwrap_or_default();
            (ok as f32 + 1.0) / ((ok + fails) as f32 + 2.0)
        };
        desync::FAKE_NAMES
            .iter()
            .enumerate()
            .filter(|(_, n)| !tried.is_some_and(|t| t.contains(n)))
            .max_by(|a, b| score(a.1).total_cmp(&score(b.1)).then_with(|| b.0.cmp(&a.0)))
            .map(|(_, n)| *n)
            .unwrap_or(desync::FAKE_NAMES[0])
    }

    fn plan(&mut self, first: &packet::Tcp, hello: &[u8], tech: &Technique, hops: Option<u8>, host: Option<&str>) -> (Option<desync::Out>, Option<&'static str>) {
        if !tech.pooled {
            return (desync::first_segment(first, hello, &self.cfg, tech, hops, &mut *self.rnd), None);
        }
        let name = self.fake_name(host);
        let blob = desync::named_fake(&self.cfg, tech, hello, name, &mut *self.rnd);
        (desync::first_segment_with(first, hello, &self.cfg, tech, hops, Some(&blob), &mut *self.rnd), Some(name))
    }

    fn rank_of(&self, name: &str) -> f32 {
        let (ok, fails) = self.rank.get(name).copied().unwrap_or_default();
        (ok as f32 + 1.0) / ((ok + fails) as f32 + 2.0)
    }

    fn next_idx(&self, host: &str, cur: usize, list: &[Technique]) -> usize {
        let n = list.len();
        let burned = self.burned.get(host);
        let fresh = |i: &usize| *i != cur && !burned.is_some_and(|b| b.contains(&list[*i].name));
        let best = (0..n)
            .filter(fresh)
            .filter(|i| list[*i].name != desync::WARP.name)
            .max_by(|a, b| self.rank_of(list[*a].name).total_cmp(&self.rank_of(list[*b].name)).then_with(|| b.cmp(a)));
        best.or_else(|| (0..n).filter(fresh).find(|i| list[*i].name == desync::WARP.name)).unwrap_or((cur + 1) % n)
    }

    fn node_learn(&mut self, addr: &IpAddr, idx: usize, outcome: Outcome) {
        let key = node_key(addr);
        match outcome {
            Outcome::Answered => {
                let m = self.nodes.entry_or(&key, || Memory { idx, ..Default::default() });
                if m.idx == idx {
                    m.ok += 1;
                    m.streak = 0;
                } else if m.ok <= 1 {
                    *m = Memory { idx, ok: 1, ..Default::default() };
                } else {
                    m.ok -= 1;
                }
            }
            Outcome::Reset | Outcome::Silent => {
                if let Some(m) = self.nodes.get_mut(&key).filter(|m| m.idx == idx) {
                    m.fails += 1;
                    m.streak += 1;
                    if m.streak >= SWITCH_AFTER {
                        m.ok = 0;
                        m.streak = 0;
                    }
                }
            }
            Outcome::Frozen => {}
        }
    }

    fn memory_for(&self, host: &str) -> Option<Memory> {
        let mut h = host;
        loop {
            if let Some(m) = self.memory.get(h) {
                return Some(*m);
            }
            let Some((_, rest)) = h.split_once('.') else { break };
            if !rest.contains('.') {
                break;
            }
            h = rest;
        }
        self.sibling_memory(host)
    }

    fn sibling_memory(&self, host: &str) -> Option<Memory> {
        let Mode::Adaptive(list) = &self.mode else { return None };
        self.memory
            .siblings(site_base(host)?)
            .filter(|(_, m)| m.ok > 0 && m.streak == 0 && list[m.idx % list.len()].name != desync::NONE.name)
            .max_by_key(|(h, m)| (m.ok, std::cmp::Reverse((*h).clone())))
            .map(|(_, m)| Memory { idx: m.idx, ..Default::default() })
    }

    pub fn memory_text(&self) -> String {
        let mut rows: Vec<(String, String)> = Vec::new();
        if let Mode::Adaptive(list) = &self.mode {
            rows.extend(self.memory.by_age().into_iter().map(|(h, m)| (h.clone(), format!("{}\t{}\t{}", list[m.idx % list.len()].name, m.ok, m.fails))));
            let mut stat: Vec<(String, String)> = list.iter().filter_map(|t| self.rank.get(t.name).filter(|r| r.0 > 0).map(|r| (format!("{STAT}{}", t.name), format!("{}\t{}\t{}", t.name, r.0, r.1)))).collect();
            stat.dedup();
            rows.extend(stat);
            rows.extend(desync::FAKE_NAMES.iter().filter_map(|n| self.name_rank.get(n).filter(|r| r.0 > 0).map(|r| (format!("{NAME}{n}"), format!("{}\t{}\t{}", desync::POOLED.name, r.0, r.1)))));
            let mut names: Vec<(&String, &&str)> = self.site_names.iter().collect();
            names.sort();
            rows.extend(names.into_iter().map(|(h, n)| (format!("{SITE_NAME}{h}"), format!("{n}\t0\t0"))));
            rows.extend(self.nodes.by_age().into_iter().filter(|(_, m)| m.ok >= NODE_TRUST).map(|(h, m)| (h.clone(), format!("{}\t{}\t{}", list[m.idx % list.len()].name, m.ok, m.fails))));
        }
        rows.extend(self.quic_memory_lines());
        let mut udp = self.udp_memory_lines();
        udp.sort();
        rows.extend(udp);
        rows.iter().map(|(h, rest)| format!("{h}\t{rest}\n")).collect()
    }

    pub fn load_memory(&mut self, text: &str) -> usize {
        let list = match &self.mode {
            Mode::Adaptive(list) => list.clone(),
            Mode::Fixed(_) => Vec::new(),
        };
        let mut n = 0;
        for line in text.lines() {
            let f: Vec<&str> = line.split('\t').collect();
            if f.len() != 4 || f[0].is_empty() {
                continue;
            }
            let (ok, fails) = (f[2].parse().unwrap_or(0), f[3].parse().unwrap_or(0));
            if f[1].starts_with("udp-") {
                n += self.udp_load(f[0], f[1], ok, fails) as usize;
                continue;
            }
            if f[1].starts_with("quic-") {
                n += self.quic_load(f[0], f[1], ok, fails) as usize;
                continue;
            }
            if let Some(h) = f[0].strip_prefix(SITE_NAME) {
                if let Some(n) = desync::FAKE_NAMES.iter().find(|n| **n == f[1]) {
                    if self.site_names.len() < MAX_MEMORY {
                        self.site_names.insert(h.to_string(), n);
                    }
                }
                continue;
            }
            if let Some(name) = f[0].strip_prefix(NAME) {
                if let Some(n) = desync::FAKE_NAMES.iter().find(|n| **n == name) {
                    self.name_rank.insert(n, (ok, fails));
                }
                continue;
            }
            let named = desync::technique(f[1]).filter(|t| t.sni.is_some() && !list.iter().any(|x| x.name == t.name) && list.iter().any(|x| x.pooled));
            let carried = named.and_then(|t| desync::FAKE_NAMES.iter().copied().find(|n| Some(*n) == t.sni));
            let wanted_name = if named.is_some() { desync::POOLED.name } else { f[1] };
            let Some(idx) = list.iter().position(|t| t.name == wanted_name) else {
                if f[1] == desync::WARP.name && self.warp.is_none() && self.later.len() < MAX_MEMORY {
                    self.later.push(line.to_string());
                }
                continue;
            };
            if f[0].starts_with(STAT) {
                self.rank.insert(list[idx].name, (ok, fails));
                continue;
            }
            if f[0].starts_with(NODE) {
                self.nodes.insert(f[0], Memory { idx, ok, fails, ..Default::default() });
                continue;
            }
            self.memory.insert(f[0], Memory { idx, ok, fails, ..Default::default() });
            if let Some(c) = carried {
                if !self.site_names.contains_key(f[0]) && self.site_names.len() < MAX_MEMORY {
                    self.site_names.insert(f[0].to_string(), c);
                }
            }
            n += 1;
        }
        n
    }

    pub fn memory_of(&self, host: &str) -> Option<(&'static str, Memory)> {
        let Mode::Adaptive(list) = &self.mode else { return None };
        self.memory.get(host).map(|m| (list[m.idx % list.len()].name, *m))
    }

    fn settle(&mut self, p: Pending, outcome: Outcome) {
        if p.tech == PROBE {
            if let Some(d) = self.recent.iter_mut().rev().find(|d| d.id == p.id) {
                d.outcome = Some(outcome);
            }
            self.probe_result(&p, outcome);
            return;
        }
        if p.bg {
            if self.notes.len() < 1000 {
                let what = if outcome == Outcome::Answered { "сервер ответил" } else { "нет ответа" };
                self.notes.push(format!("проверка самоподстройки: {} — приём {}, {what}", p.host.as_deref().unwrap_or("без имени"), p.tech));
            }
        } else {
            match outcome {
                Outcome::Answered => self.stats.answered += 1,
                Outcome::Reset => self.stats.reset += 1,
                Outcome::Silent => self.stats.silent += 1,
                Outcome::Frozen => self.stats.frozen += 1,
            }
            let t = self.stats.by_tech.entry(p.tech).or_default();
            match outcome {
                Outcome::Answered => t.answered += 1,
                Outcome::Reset => t.reset += 1,
                Outcome::Silent => t.silent += 1,
                Outcome::Frozen => t.frozen += 1,
            }
        }
        if let Some(d) = self.recent.iter_mut().rev().find(|d| d.id == p.id) {
            d.outcome = Some(outcome);
        }
        let rescue = p.host.as_deref().is_some_and(|h| self.fail_log.get(h).is_some_and(|l| l.iter().any(|e| e.1 != p.tech)));
        let failed_before = p.host.as_deref().is_some_and(|h| self.fail_log.get(h).is_some_and(|l| !l.is_empty()));
        if let Some(h) = p.host.as_deref().filter(|_| !p.tech.starts_with("quic-")) {
            self.note_outcome(h, p.tech, p.since, outcome, !p.bg);
        }
        if p.tech.starts_with("quic-") {
            self.quic_learn(&p, outcome);
        } else if p.tech.starts_with("udp-") {
            self.udp_learn(&p, outcome);
        } else if let (Mode::Adaptive(list), Some(h)) = (&self.mode, &p.host) {
            let n = list.len();
            let cur = list.iter().position(|t| t.name == p.tech);
            if let Some(c) = cur.filter(|_| rescue && outcome != Outcome::Frozen) {
                let r = self.rank.entry(list[c].name).or_default();
                if outcome == Outcome::Answered {
                    r.0 = r.0.saturating_add(1);
                } else {
                    r.1 = r.1.saturating_add(1);
                }
            }
            let next = cur.map(|c| self.next_idx(h, c, list));
            let pooled = cur.is_some_and(|c| list[c].pooled);
            let mut name_hold = false;
            if let (true, Some(name)) = (pooled, p.fname) {
                if failed_before && outcome != Outcome::Frozen {
                    let r = self.name_rank.entry(name).or_default();
                    if outcome == Outcome::Answered {
                        r.0 = r.0.saturating_add(1);
                    } else {
                        r.1 = r.1.saturating_add(1);
                    }
                }
                match outcome {
                    Outcome::Answered => {
                        if self.site_names.len() >= MAX_MEMORY && !self.site_names.contains_key(h.as_str()) {
                            self.site_names.clear();
                        }
                        self.site_names.insert(h.clone(), name);
                        self.tried_names.remove(h.as_str());
                    }
                    Outcome::Frozen => {}
                    _ => {
                        if self.site_names.get(h.as_str()) == Some(&name) {
                            self.site_names.remove(h.as_str());
                        }
                        if self.tried_names.len() >= MAX_MEMORY && !self.tried_names.contains_key(h.as_str()) {
                            self.tried_names.clear();
                        }
                        let t = self.tried_names.entry(h.clone()).or_default();
                        if !t.contains(&name) {
                            t.push(name);
                        }
                        name_hold = t.len() < desync::NAME_TRIES;
                    }
                }
            }
            let mut switched = false;
            {
                let base = self.memory_for(h).map(|m| Memory { idx: m.idx, ..Default::default() }).or_else(|| self.node_hint(p.node.as_ref())).unwrap_or_default();
                let m = self.memory.entry_or(h, || base);
                if outcome == Outcome::Answered {
                    m.ok += 1;
                    if cur == Some(m.idx % n) {
                        m.streak = 0;
                    }
                } else {
                    m.fails += 1;
                    if cur == Some(m.idx % n) {
                        m.streak += 1;
                        if pooled && name_hold && !m.hint {
                            m.streak = 0;
                        }
                        if m.streak >= SWITCH_AFTER || (pooled && !name_hold && !m.hint) {
                            m.idx = if m.hint { 0 } else { next.unwrap_or((m.idx + 1) % n) };
                            m.hint = false;
                            m.streak = 0;
                            switched = true;
                        }
                    }
                }
            }
            if switched && pooled {
                self.tried_names.remove(h.as_str());
            }
            if let Some(c) = cur.filter(|_| switched) {
                if self.burned.len() >= MAX_MEMORY && !self.burned.contains_key(h.as_str()) {
                    self.burned.clear();
                }
                let b = self.burned.entry(h.clone()).or_default();
                if !b.contains(&list[c].name) {
                    b.push(list[c].name);
                }
                if b.len() >= n {
                    self.burned.remove(h.as_str());
                }
            }
            if let (Some(ip), Some(c)) = (p.node, cur) {
                if list[c].name != desync::WARP.name {
                    self.node_learn(&ip, c, outcome);
                }
            }
        }
        if self.verbose {
            let host = p.host.as_deref().unwrap_or("без имени");
            let what = match outcome {
                Outcome::Answered => "сервер ответил",
                Outcome::Reset => "пришёл сброс",
                Outcome::Silent => "нет ответа",
                Outcome::Frozen => "сервер оборвал загрузку",
            };
            self.note(format!("итог {host} ({}): {what}", p.tech));
        }
    }

    pub fn observe(&mut self, raw: &[u8], now: Instant) {
        if packet::is_udp(raw) {
            self.observe_udp(raw);
            return;
        }
        let Some(t) = packet::parse(raw) else { return };
        let key = (t.dst, t.dport, t.src, t.sport);
        if t.has(SYN) && t.flags & 0x10 != 0 {
            if let Some(f) = self.flows.get_mut(&key) {
                f.hops = Some(packet::hops(t.ttl));
                f.srv_isn = Some(t.seq);
            }
            return;
        }
        let outcome = if t.has(RST) {
            Outcome::Reset
        } else {
            let d = t.payload();
            if d.len() >= 6 && d[0] == 0x16 && d[1] == 0x03 && d[5] == 0x02 {
                Outcome::Answered
            } else {
                return;
            }
        };
        let _ = now;
        if let Some(p) = self.pending.remove(&key) {
            if outcome == Outcome::Reset && !p.bg && p.tech != PROBE {
                self.classify_reset(&key, &p, t.ttl);
            }
            self.settle(p, outcome);
        }
    }

    fn classify_reset(&mut self, key: &Key, p: &Pending, ttl: u8) {
        let rst = packet::hops(ttl);
        let server = self.flows.get(key).and_then(|f| f.hops);
        let from = reset_from(rst, server);
        match from {
            RstFrom::Filter => self.stats.rst_filter += 1,
            RstFrom::Server => self.stats.rst_server += 1,
            RstFrom::Far => self.stats.rst_far += 1,
            RstFrom::Blind => self.stats.rst_blind += 1,
        }
        let Some(host) = p.host.as_deref() else { return };
        if from == RstFrom::Blind || self.rst_noted.len() >= RST_NOTES || !self.rst_noted.insert(host.to_string()) {
            return;
        }
        let who = match from {
            RstFrom::Filter => "от фильтра на пути",
            RstFrom::Server => "от самого сервера",
            _ => "дальше сервера",
        };
        let server = server.map(|s| s.to_string()).unwrap_or_default();
        self.notes.push(format!("сброс: {host} — {who} (узлов до сброса {rst}, до сервера {server}), приём {}", p.tech));
    }

    pub fn pending(&self) -> usize {
        self.pending.len()
    }

    pub fn with_clock(mut self, clock: fn() -> String) -> Self {
        self.clock = clock;
        self
    }

    fn record(&mut self, host: &Option<String>, owner: &Option<Owner>, handled: bool, why: &'static str, technique: Option<&'static str>) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        if !self.track {
            return id;
        }
        if !handled {
            let app = owner.as_ref().and_then(|o| o.name.as_deref());
            if app.is_some_and(|a| QUIET_APPS.contains(&a)) {
                return id;
            }
            let same = self.recent.iter_mut().rev().take(SAME_SKIP).find(|d| !d.handled && d.why == why && d.host == *host && d.app.as_deref() == app);
            if let Some(d) = same {
                d.time = (self.clock)();
                d.outcome = None;
                self.next_id -= 1;
                return d.id;
            }
        }
        if self.recent.len() >= RECENT {
            self.recent.pop_front();
        }
        self.recent.push_back(Decision {
            id,
            technique,
            outcome: None,
            time: (self.clock)(),
            host: host.clone(),
            app: owner.as_ref().and_then(|o| o.name.clone()),
            pid: owner.as_ref().map(|o| o.pid),
            parts: 0,
            handled,
            why,
        });
        id
    }

    fn note(&mut self, text: String) {
        if self.verbose && self.notes.len() < 1000 {
            self.notes.push(text);
        }
    }

    pub fn lists_mut(&mut self) -> &mut Lists {
        &mut self.lists
    }

    pub fn flows(&self) -> usize {
        self.flows.len()
    }

    fn excluded(&self, host: Option<&str>, dst: &IpAddr) -> bool {
        host.is_some_and(|h| self.lists.host_excludes.matches(h)) || self.lists.ip_excludes.contains(dst)
    }

    pub fn handle(&mut self, raw: &[u8], meta: A, now: Instant) -> Action<A> {
        if packet::is_udp(raw) {
            return self.handle_udp(raw, meta, now);
        }
        let Some(t) = packet::parse(raw) else { return Action::Pass };
        let key = (t.src, t.sport, t.dst, t.dport);
        if self.forward && t.flags & 0x10 != 0 {
            let answered = self.pending.get(&key).and_then(|p| p.ack0).is_some_and(|a| (t.ack.wrapping_sub(a) as i32) > 0);
            if answered {
                if let Some(p) = self.pending.remove(&key) {
                    self.settle(p, Outcome::Answered);
                }
            }
        }
        if t.has(FIN) && !t.has(RST) && self.flows.get(&key).is_some_and(|f| matches!(f.stage, Stage::Done) && f.watch.is_some()) {
            if let Some(f) = self.flows.get_mut(&key) {
                f.last = now;
            }
            self.watch_out(&key, t.seq, t.ack, t.payload().len() as u32 + 1, now);
            return Action::Pass;
        }
        if t.has(RST) || t.has(FIN) {
            return match self.flows.remove(&key).map(|f| f.stage) {
                Some(Stage::Reasm { held, .. }) => {
                    self.stats.flushed += 1;
                    let mut out = held;
                    out.push((packet::fixed(raw), meta));
                    Action::Send(out)
                }
                _ => Action::Pass,
            };
        }
        if t.has(SYN) {
            if self.flows.len() >= MAX_FLOWS {
                self.evict(now);
            }
            self.flows.insert(key, Flow { isn: Some(t.seq), stage: Stage::Fresh, last: now, hops: None, srv_isn: None, watch: None, sent: None, redo: None, resent: 0, wscale: packet::wscale(&t), wss: 0 });
            return Action::Pass;
        }
        let data = t.payload();
        if data.is_empty() {
            if let Some(f) = self.flows.get_mut(&key).filter(|f| f.wss > 0) {
                f.wss -= 1;
                f.last = now;
                return Action::Send(vec![(packet::with_window(raw, desync::wss_window(f.wscale)), meta)]);
            }
            return Action::Pass;
        }
        let tls_port = self.tls_port(t.dport);
        if !self.flows.contains_key(&key) {
            if !tls_port || !tls::is_client_hello(data) {
                return Action::Pass;
            }
            if self.flows.len() >= MAX_FLOWS {
                self.evict(now);
            }
            self.flows.insert(key, Flow { isn: None, stage: Stage::Fresh, last: now, hops: None, srv_isn: None, watch: None, sent: None, redo: None, resent: 0, wscale: None, wss: 0 });
        }
        let flow = self.flows.get_mut(&key).unwrap();
        flow.last = now;
        let stage = std::mem::replace(&mut flow.stage, Stage::Done);
        match stage {
            Stage::Done => {
                let again = self.flows.get(&key).and_then(|f| f.sent).filter(|(seq, _)| *seq == t.seq);
                if let Some((_, tech)) = again.filter(|_| tls_port && tls::is_client_hello(data)) {
                    return self.resend_hello(key, tech, raw, t.seq, data, meta, now);
                }
                if let Some(f) = self.flows.get_mut(&key) {
                    f.wss = 0;
                }
                if data.len() > 1 {
                    self.watch_out(&key, t.seq, t.ack, data.len() as u32, now);
                }
                Action::Pass
            }
            Stage::Fresh => {
                let start = flow.isn.is_none_or(|isn| t.seq == isn.wrapping_add(1));
                if !start {
                    return Action::Pass;
                }
                if !tls_port || !tls::is_client_hello(data) {
                    return self.handle_plain(raw, meta, now);
                }
                let Some(need) = tls::record_len(data).filter(|n| *n <= MAX_REASM) else { return Action::Pass };
                if data.len() >= need {
                    return self.decide(&data[..need], vec![(packet::fixed(raw), meta)], now);
                }
                let next_seq = t.seq.wrapping_add(data.len() as u32);
                let flow = self.flows.get_mut(&key).unwrap();
                flow.stage = Stage::Reasm { need, buf: data.to_vec(), held: vec![(packet::fixed(raw), meta)], next_seq, since: now };
                Action::Hold
            }
            Stage::Reasm { need, mut buf, mut held, next_seq, since } => {
                let flow = self.flows.get_mut(&key).unwrap();
                if t.seq != next_seq {
                    let behind = next_seq.wrapping_sub(t.seq);
                    if behind != 0 && behind < 0x8000_0000 && (behind as usize) <= buf.len() {
                        flow.stage = Stage::Reasm { need, buf, held, next_seq, since };
                        return Action::Drop;
                    }
                    self.stats.flushed += 1;
                    held.push((packet::fixed(raw), meta));
                    return Action::Send(held);
                }
                buf.extend_from_slice(data);
                held.push((packet::fixed(raw), meta));
                let next_seq = next_seq.wrapping_add(data.len() as u32);
                if buf.len() >= need {
                    buf.truncate(need);
                    if let Some(tech) = flow.redo.take() {
                        return self.redo(&key, tech, &buf, held);
                    }
                    return self.decide(&buf, held, now);
                }
                flow.stage = Stage::Reasm { need, buf, held, next_seq, since };
                Action::Hold
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn resend_hello(&mut self, key: Key, tech: &'static str, raw: &[u8], seq: u32, data: &[u8], meta: A, now: Instant) -> Action<A> {
        let Some(need) = tls::record_len(data).filter(|n| *n <= MAX_REASM) else { return Action::Pass };
        let Some(f) = self.flows.get_mut(&key) else { return Action::Pass };
        if f.resent >= RESEND_MAX {
            return Action::Pass;
        }
        f.resent += 1;
        if data.len() >= need {
            return self.redo(&key, tech, &data[..need], vec![(packet::fixed(raw), meta)]);
        }
        f.redo = Some(tech);
        f.stage = Stage::Reasm { need, buf: data.to_vec(), held: vec![(packet::fixed(raw), meta)], next_seq: seq.wrapping_add(data.len() as u32), since: now };
        Action::Hold
    }

    fn redo(&mut self, key: &Key, tech: &'static str, hello: &[u8], held: Vec<(Vec<u8>, A)>) -> Action<A> {
        let raw0 = held[0].0.clone();
        let Some(first) = packet::parse(&raw0) else { return Action::Send(held) };
        let technique = desync::technique(tech).unwrap_or(desync::GENERAL);
        let hops = self.flows.get(key).and_then(|f| f.hops);
        let host = tls::sni(hello);
        let Some(out) = self.plan(&first, hello, &technique, hops, host.as_deref()).0 else { return Action::Send(held) };
        self.stats.resent += 1;
        let meta = held[0].1.clone();
        let mut sends: Vec<(Vec<u8>, A)> = out.packets.into_iter().map(|p| (p, meta.clone())).collect();
        sends.extend(held.into_iter().skip(1));
        Action::Send(sends)
    }

    fn tls_port(&self, port: u16) -> bool {
        let own = self.lists.own;
        port == 443 || (own.cf && profiles::CF_PORTS.contains(&port)) || (own.myip_tcp && port == 8443)
    }

    fn handle_plain(&mut self, raw: &[u8], meta: A, now: Instant) -> Action<A> {
        let Some(t) = packet::parse(raw) else { return Action::Pass };
        let data = t.payload();
        let own = self.lists.own;
        let http = t.dport == 80 && profiles::is_http_request(data) && (own.http || own.myip_tcp);
        let game = !http
            && own.game_tcp
            && self.lists.game_tcp.contains(t.dport)
            && self.lists.ipset.contains(&t.dst)
            && !self.lists.ip_excludes.contains(&t.dst);
        if (!http && !game) || self.cfg.fake_http.is_empty() {
            return Action::Pass;
        }
        let host = if http { profiles::http_host(data) } else { None };
        let owner = self.owner_of(t.src, t.sport, t.dst, t.dport, now);
        let (wanted, why) = if http {
            let (w, why, _) = self.verdict(host.as_deref(), &t.dst, 80, false, &owner);
            (w, why)
        } else {
            match self.lists.apps.rule(owner.as_ref().and_then(|o| o.name.as_deref())) {
                AppRule::Skip => (false, "программа в исключениях"),
                _ => (true, "игра: адрес в «Мои IP»"),
            }
        };
        let who = Self::who(&host, &owner);
        if !wanted {
            if http && why == "сайта нет в списке" {
                return Action::Pass;
            }
            self.stats.skipped += 1;
            self.record(&host, &owner, false, why, None);
            self.note(format!("пропущен {who}: {why}"));
            return Action::Pass;
        }
        let tech = self.choose(host.as_deref(), None);
        if tech.name == desync::NONE.name {
            self.record(&host, &owner, true, why, Some(tech.name));
            self.note(format!("пропущен как есть {who}: {why}, приём {}", tech.name));
            self.stats.by_tech.entry(tech.name).or_default().used += 1;
            return Action::Pass;
        }
        let key = (t.src, t.sport, t.dst, t.dport);
        let hops = self.flows.get(&key).and_then(|f| f.hops);
        let blob = self.cfg.fake_http.clone();
        let Some(out) = desync::first_segment_with(&t, data, &self.cfg, &tech, hops, Some(&blob), &mut *self.rnd) else { return Action::Pass };
        self.record(&host, &owner, true, why, Some(tech.name));
        self.note(format!("обработан {who}: {why}, приём {}", tech.name));
        self.stats.handled += 1;
        self.stats.by_tech.entry(tech.name).or_default().used += 1;
        if t.ts.is_none() {
            self.stats.no_ts += 1;
        }
        if out.fallback.is_some() {
            self.stats.fallback += 1;
        }
        Action::Send(out.packets.into_iter().map(|p| (p, meta.clone())).collect())
    }

    fn mark_parts(&mut self, id: u64, n: usize) {
        if let Some(d) = self.recent.iter_mut().rev().find(|d| d.id == id) {
            d.parts = n.min(255) as u8;
        }
    }

    fn decide(&mut self, hello: &[u8], held: Vec<(Vec<u8>, A)>, now: Instant) -> Action<A> {
        let single = held.len() == 1;
        let parts = held.len();
        let first_raw = held[0].0.clone();
        let first = packet::parse(&first_raw).expect("held packets are parsed");
        let host = tls::sni(hello);
        let by_port = !self.forward && self.bg_ports.contains(&first.sport);
        let found = if by_port { None } else { self.owner_of(first.src, first.sport, first.dst, first.dport, now) };
        let own = !self.forward && found.as_ref().is_some_and(|o| o.pid == std::process::id());
        let bg = by_port || own;
        let owner = if bg { Some(Owner { pid: std::process::id(), name: Some(BG_APP.to_string()), source: Source::Table }) } else { found };
        let (wanted, why, zero) = self.verdict(host.as_deref(), &first.dst, first.dport, false, &owner);
        let (wanted, why) = if by_port { (true, BG_APP) } else { (wanted, why) };
        if wanted && !bg && first.dport == 443 {
            if let Some(h) = &host {
                self.note_target(h, SocketAddr::new(first.dst, first.dport), parts, now);
            }
        }
        let tech = self.choose(host.as_deref(), Some(&first.dst));
        let key = (first.src, first.sport, first.dst, first.dport);
        let who = Self::who(&host, &owner);
        let forced = !bg && first.dport == 443 && host.as_deref().is_some_and(|h| self.warp.as_ref().is_some_and(|w| w.forced(h)));
        let (wanted, tech) = if forced { (true, desync::WARP) } else { (wanted, tech) };
        if wanted && !bg && tech.name == desync::WARP.name {
            if let Some(w) = self.warp.clone() {
                w.want(now);
                if w.ready() && !w.has_ip(&first.dst) {
                    if let Some(rst) = crate::warp::reset_for(&first) {
                        w.add_ip(first.dst, now);
                        self.notes.push(format!("WARP: {who} — следующее соединение пойдёт через WARP"));
                        return Action::Reject(rst, held[0].1.clone());
                    }
                }
            }
        }
        let tech = if tech.name == desync::WARP.name { desync::GENERAL } else { tech };
        if wanted && tech.name == desync::NONE.name {
            let id = self.record(&host, &owner, true, why, Some(tech.name));
            self.mark_parts(id, parts);
            self.note(format!("пропущен как есть {who}: {why}, приём {}", tech.name));
            if !bg {
                self.stats.by_tech.entry(tech.name).or_default().used += 1;
            }
            if first.dport == 443 {
                self.pending.insert(key, Pending { host: host.clone(), tech: tech.name, since: now, id, ack0: Some(first.ack), dcid: None, bg, node: Some(first.dst), fname: None });
                if !bg {
                    self.start_watch(&key, &host, tech.name, id, first.dst, first.seq.wrapping_add(hello.len() as u32), now);
                }
            }
            return if single { Action::Pass } else { Action::Send(held) };
        }
        let hops = self.flows.get(&key).and_then(|f| f.hops);
        let (plan, fname) = if wanted { self.plan(&first, hello, &tech, hops, host.as_deref()) } else { (None, None) };
        let Some(out) = plan else {
            self.stats.skipped += 1;
            let id = self.record(&host, &owner, false, why, None);
            self.note(format!("пропущен {who}: {why}"));
            if self.autolist && !wanted && first.dport == 443 && host.is_some() && why == "сайта нет в списке" {
                self.pending.insert(key, Pending { host: host.clone(), tech: PROBE, since: now, id, ack0: Some(first.ack), dcid: None, bg: false, node: None, fname: None });
            }
            return if single { Action::Pass } else { Action::Send(held) };
        };
        let id = self.record(&host, &owner, true, why, Some(tech.name));
        self.mark_parts(id, parts);
        if let Some(f) = self.flows.get_mut(&key) {
            f.sent = Some((first.seq, tech.name));
        }
        match out.fallback {
            Some(f) => self.note(format!("обработан {who}: {why}, приём {}, запасной {f}", tech.name)),
            None => self.note(format!("обработан {who}: {why}, приём {}", tech.name)),
        }
        if !bg {
            self.stats.handled += 1;
            self.stats.by_tech.entry(tech.name).or_default().used += 1;
        }
        if first.dport == 443 {
            self.pending.insert(key, Pending { host: host.clone(), tech: tech.name, since: now, id, ack0: Some(first.ack), dcid: None, bg, node: Some(first.dst), fname });
            if !bg {
                self.start_watch(&key, &host, tech.name, id, first.dst, first.seq.wrapping_add(hello.len() as u32), now);
            }
        }
        if let Some(n) = fname {
            self.note(format!("имя в фейке {who}: {n}"));
        }
        if first.ts.is_none() {
            self.stats.no_ts += 1;
        }
        if out.fallback.is_some() {
            self.stats.fallback += 1;
        }
        let meta = held[0].1.clone();
        let mut packets = out.packets;
        let small = tech.wssize.then(|| desync::wss_window(self.flows.get(&key).and_then(|f| f.wscale)));
        if let Some(w) = small {
            for p in packets.iter_mut() {
                *p = packet::with_window(p, w);
            }
            if let Some(f) = self.flows.get_mut(&key) {
                f.wss = desync::WSS_PACKETS;
            }
        }
        if zero && !first.v6 && !self.forward {
            for p in packets.iter_mut() {
                *p = packet::with_ip_id(p, 0);
            }
        }
        let mut sends: Vec<(Vec<u8>, A)> = packets.into_iter().map(|p| (p, meta.clone())).collect();
        let mut ip_id = out.next_ip_id;
        for (raw, m) in held.into_iter().skip(1) {
            let raw = match small {
                Some(w) => packet::with_window(&raw, w),
                None => raw,
            };
            if first.v6 || zero {
                sends.push((raw, m));
            } else {
                sends.push((packet::with_ip_id(&raw, ip_id), m));
                ip_id = desync::next_ip_id(ip_id);
            }
        }
        if tech.pause_ms > 0 && out.fakes > 0 && out.fakes < sends.len() && self.delayed.len() < MAX_FLOWS {
            let later = sends.split_off(out.fakes);
            self.delayed.push((now + Duration::from_millis(tech.pause_ms as u64), later));
        }
        Action::Send(sends)
    }

    pub fn expire(&mut self, now: Instant) -> Vec<(Vec<u8>, A)> {
        let mut due = Vec::new();
        self.delayed.retain_mut(|(at, pkts)| {
            if *at <= now {
                due.append(pkts);
                false
            } else {
                true
            }
        });
        let mut quic_out = self.expire_quic(now);
        self.expire_udp(now);
        let late: Vec<Key> = self.pending.iter().filter(|(_, p)| now.duration_since(p.since) >= SILENT).map(|(k, _)| *k).collect();
        for k in late {
            if let Some(p) = self.pending.remove(&k) {
                self.settle(p, Outcome::Silent);
            }
        }
        let mut out = Vec::new();
        let mut gone = Vec::new();
        for (k, f) in self.flows.iter_mut() {
            if let Stage::Reasm { since, .. } = &f.stage {
                if now.duration_since(*since) >= REASM_WAIT {
                    if let Stage::Reasm { held, .. } = std::mem::replace(&mut f.stage, Stage::Done) {
                        self.stats.flushed += 1;
                        out.extend(held);
                    }
                }
            } else if now.duration_since(f.last) >= IDLE {
                gone.push(*k);
            }
        }
        for k in gone {
            self.flows.remove(&k);
        }
        out.append(&mut quic_out);
        due.append(&mut out);
        due
    }

    fn evict(&mut self, now: Instant) {
        let mut victims: Vec<(Instant, Key)> =
            self.flows.iter().filter(|(_, f)| !matches!(f.stage, Stage::Reasm { .. })).map(|(k, f)| (f.last.min(now), *k)).collect();
        victims.sort_by_key(|(t, _)| *t);
        let victims: Vec<Key> = victims.into_iter().take(MAX_FLOWS / 4).map(|(_, k)| k).collect();
        for k in victims {
            self.flows.remove(&k);
        }
    }

    pub fn drain(&mut self) -> Vec<(Vec<u8>, A)> {
        let mut out: Vec<(Vec<u8>, A)> = self.delayed.drain(..).flat_map(|(_, p)| p).collect();
        for (_, f) in self.flows.drain() {
            if let Stage::Reasm { held, .. } = f.stage {
                out.extend(held);
            }
        }
        out.extend(self.drain_quic());
        out
    }
}
