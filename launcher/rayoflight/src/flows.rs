use crate::desync::{self, Config, Technique};
use crate::lists::{Hosts, Nets};
use crate::packet::{self, FIN, RST, SYN};
use crate::procs::{AppRule, Apps, Conn, Owner, Source};
use crate::profiles::{self, Own, Ports};
use crate::tls;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::net::IpAddr;
use std::time::{Duration, Instant};

mod quicflow;
pub use quicflow::{QuicCfg, QuicMode};

pub const MAX_REASM: usize = 16384;
pub const REASM_WAIT: Duration = Duration::from_millis(300);
pub const IDLE: Duration = Duration::from_secs(60);
pub const MAX_FLOWS: usize = 4096;
pub const SILENT: Duration = Duration::from_secs(4);
pub const MAX_MEMORY: usize = 4096;
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
}

#[derive(Debug, PartialEq, Eq)]
pub enum Action<A> {
    Pass,
    Hold,
    Drop,
    Send(Vec<(Vec<u8>, A)>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Answered,
    Reset,
    Silent,
}

impl Outcome {
    pub fn name(self) -> &'static str {
        match self {
            Outcome::Answered => "answered",
            Outcome::Reset => "reset",
            Outcome::Silent => "silent",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TechStats {
    pub used: u64,
    pub answered: u64,
    pub reset: u64,
    pub silent: u64,
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
    pub app_skipped: u64,
    pub app_all: u64,
    pub answered: u64,
    pub reset: u64,
    pub silent: u64,
    pub by_tech: BTreeMap<&'static str, TechStats>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mode {
    Fixed(Technique),
    Adaptive(Vec<Technique>),
}

impl Mode {
    pub fn parse(name: &str) -> Option<Mode> {
        if name.eq_ignore_ascii_case("adaptive") {
            return Some(Mode::Adaptive(vec![
                desync::GENERAL,
                desync::SNI_SPLIT,
                desync::NONE,
                desync::TTL,
                desync::DISORDER,
                desync::MD5SIG,
                desync::BADSEQ,
                desync::RANDOM,
            ]));
        }
        desync::technique(name).map(Mode::Fixed)
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
}

pub const SWITCH_AFTER: u32 = 2;

struct Pending {
    host: Option<String>,
    tech: &'static str,
    since: Instant,
    id: u64,
    ack0: Option<u32>,
    dcid: Option<Vec<u8>>,
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

pub struct Engine<A> {
    cfg: Config,
    lists: Lists,
    flows: HashMap<Key, Flow<A>>,
    rnd: Box<dyn FnMut(&mut [u8]) + Send>,
    app_of: Option<AppLookup>,
    mode: Mode,
    pending: HashMap<Key, Pending>,
    memory: HashMap<String, Memory>,
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
    qmemory: HashMap<String, Memory>,
    auto_fails: HashMap<String, (u32, Instant)>,
    auto_new: Vec<String>,
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
            memory: HashMap::new(),
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
            qmemory: HashMap::new(),
            auto_fails: HashMap::new(),
            auto_new: Vec::new(),
        }
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

    pub fn with_mode(mut self, mode: Mode) -> Self {
        self.mode = mode;
        self
    }

    pub fn mode(&self) -> &Mode {
        &self.mode
    }

    fn choose(&mut self, host: Option<&str>) -> Technique {
        match &self.mode {
            Mode::Fixed(t) => *t,
            Mode::Adaptive(list) => {
                let m = self.memory.get(host.unwrap_or("")).copied().unwrap_or_default();
                list[m.idx % list.len()]
            }
        }
    }

    pub fn memory_text(&self) -> String {
        let mut rows: Vec<(String, String)> = Vec::new();
        if let Mode::Adaptive(list) = &self.mode {
            rows.extend(self.memory.iter().map(|(h, m)| (h.clone(), format!("{}\t{}\t{}", list[m.idx % list.len()].name, m.ok, m.fails))));
        }
        rows.extend(self.quic_memory_lines());
        rows.sort();
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
            if f[1].starts_with("quic-") {
                n += self.quic_load(f[0], f[1], ok, fails) as usize;
                continue;
            }
            if self.memory.len() >= MAX_MEMORY {
                continue;
            }
            let Some(idx) = list.iter().position(|t| t.name == f[1]) else { continue };
            self.memory.insert(f[0].to_string(), Memory { idx, ok, fails, streak: 0 });
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
        match outcome {
            Outcome::Answered => self.stats.answered += 1,
            Outcome::Reset => self.stats.reset += 1,
            Outcome::Silent => self.stats.silent += 1,
        }
        let t = self.stats.by_tech.entry(p.tech).or_default();
        match outcome {
            Outcome::Answered => t.answered += 1,
            Outcome::Reset => t.reset += 1,
            Outcome::Silent => t.silent += 1,
        }
        if let Some(d) = self.recent.iter_mut().rev().find(|d| d.id == p.id) {
            d.outcome = Some(outcome);
        }
        if p.tech.starts_with("quic-") {
            self.quic_learn(&p, outcome);
        } else if let (Mode::Adaptive(list), Some(h)) = (&self.mode, &p.host) {
            let n = list.len();
            let cur = list.iter().position(|t| t.name == p.tech);
            if self.memory.len() < MAX_MEMORY || self.memory.contains_key(h) {
                let m = self.memory.entry(h.clone()).or_default();
                if outcome == Outcome::Answered {
                    m.ok += 1;
                    if cur == Some(m.idx % n) {
                        m.streak = 0;
                    }
                } else {
                    m.fails += 1;
                    if cur == Some(m.idx % n) {
                        m.streak += 1;
                        if m.streak >= SWITCH_AFTER {
                            m.idx = (m.idx + 1) % n;
                            m.streak = 0;
                        }
                    }
                }
            }
        }
        if self.verbose {
            let host = p.host.as_deref().unwrap_or("без имени");
            let what = match outcome {
                Outcome::Answered => "сервер ответил",
                Outcome::Reset => "пришёл сброс",
                Outcome::Silent => "нет ответа",
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
            self.settle(p, outcome);
        }
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
            self.flows.insert(key, Flow { isn: Some(t.seq), stage: Stage::Fresh, last: now, hops: None });
            return Action::Pass;
        }
        let data = t.payload();
        if data.is_empty() {
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
            self.flows.insert(key, Flow { isn: None, stage: Stage::Fresh, last: now, hops: None });
        }
        let flow = self.flows.get_mut(&key).unwrap();
        flow.last = now;
        let stage = std::mem::replace(&mut flow.stage, Stage::Done);
        match stage {
            Stage::Done => Action::Pass,
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
                    return self.decide(&buf, held, now);
                }
                flow.stage = Stage::Reasm { need, buf, held, next_seq, since };
                Action::Hold
            }
        }
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
        let tech = self.choose(host.as_deref());
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
        let owner = self.owner_of(first.src, first.sport, first.dst, first.dport, now);
        let (wanted, why, zero) = self.verdict(host.as_deref(), &first.dst, first.dport, false, &owner);
        let tech = self.choose(host.as_deref());
        let key = (first.src, first.sport, first.dst, first.dport);
        let who = Self::who(&host, &owner);
        if wanted && tech.name == desync::NONE.name {
            let id = self.record(&host, &owner, true, why, Some(tech.name));
            self.mark_parts(id, parts);
            self.note(format!("пропущен как есть {who}: {why}, приём {}", tech.name));
            self.stats.by_tech.entry(tech.name).or_default().used += 1;
            if first.dport == 443 {
                self.pending.insert(key, Pending { host: host.clone(), tech: tech.name, since: now, id, ack0: Some(first.ack), dcid: None });
            }
            return if single { Action::Pass } else { Action::Send(held) };
        }
        let hops = self.flows.get(&key).and_then(|f| f.hops);
        let plan = if wanted { desync::first_segment(&first, hello, &self.cfg, &tech, hops, &mut *self.rnd) } else { None };
        let Some(out) = plan else {
            self.stats.skipped += 1;
            let id = self.record(&host, &owner, false, why, None);
            self.note(format!("пропущен {who}: {why}"));
            if self.autolist && !wanted && first.dport == 443 && host.is_some() && why == "сайта нет в списке" {
                self.pending.insert(key, Pending { host: host.clone(), tech: PROBE, since: now, id, ack0: Some(first.ack), dcid: None });
            }
            return if single { Action::Pass } else { Action::Send(held) };
        };
        let id = self.record(&host, &owner, true, why, Some(tech.name));
        self.mark_parts(id, parts);
        match out.fallback {
            Some(f) => self.note(format!("обработан {who}: {why}, приём {}, запасной {f}", tech.name)),
            None => self.note(format!("обработан {who}: {why}, приём {}", tech.name)),
        }
        self.stats.handled += 1;
        self.stats.by_tech.entry(tech.name).or_default().used += 1;
        if first.dport == 443 {
            self.pending.insert(key, Pending { host: host.clone(), tech: tech.name, since: now, id, ack0: Some(first.ack), dcid: None });
        }
        if first.ts.is_none() {
            self.stats.no_ts += 1;
        }
        if out.fallback.is_some() {
            self.stats.fallback += 1;
        }
        let meta = held[0].1.clone();
        let mut packets = out.packets;
        if zero && !first.v6 && !self.forward {
            for p in packets.iter_mut() {
                *p = packet::with_ip_id(p, 0);
            }
        }
        let mut sends: Vec<(Vec<u8>, A)> = packets.into_iter().map(|p| (p, meta.clone())).collect();
        let mut ip_id = out.next_ip_id;
        for (raw, m) in held.into_iter().skip(1) {
            if first.v6 || zero {
                sends.push((raw, m));
            } else {
                sends.push((packet::with_ip_id(&raw, ip_id), m));
                ip_id = desync::next_ip_id(ip_id);
            }
        }
        Action::Send(sends)
    }

    pub fn expire(&mut self, now: Instant) -> Vec<(Vec<u8>, A)> {
        let mut quic_out = self.expire_quic(now);
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
        out
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
        let mut out = Vec::new();
        for (_, f) in self.flows.drain() {
            if let Stage::Reasm { held, .. } = f.stage {
                out.extend(held);
            }
        }
        out.extend(self.drain_quic());
        out
    }
}
