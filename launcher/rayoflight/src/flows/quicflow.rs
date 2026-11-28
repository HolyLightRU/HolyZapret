use super::{Action, Engine, IDLE, Key, Memory, Outcome, Pending, REASM_WAIT, SWITCH_AFTER};
use crate::desync::{SNI_POOL, pick};
use crate::{packet, profiles, quic, tls};
use std::time::Instant;

const MAX_HELD: usize = 4;
const PIECES: [usize; 8] = [8, 12, 16, 24, 32, 48, 64, 128];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuicMode {
    Fake,
    Random,
    Shuffle,
    Block,
    Pass,
    Adaptive,
    Split,
}

pub const ROTATION: [QuicMode; 4] = [QuicMode::Fake, QuicMode::Random, QuicMode::Pass, QuicMode::Block];

impl QuicMode {
    pub const NAMES: [&'static str; 6] = ["general", "random", "shuffle", "block", "adaptive", "split"];

    pub fn parse(s: &str) -> Option<QuicMode> {
        match s.to_ascii_lowercase().as_str() {
            "general" => Some(QuicMode::Fake),
            "random" => Some(QuicMode::Random),
            "shuffle" => Some(QuicMode::Shuffle),
            "block" => Some(QuicMode::Block),
            "adaptive" => Some(QuicMode::Adaptive),
            "split" => Some(QuicMode::Split),
            _ => None,
        }
    }

    pub fn tech(self) -> &'static str {
        match self {
            QuicMode::Fake => "quic-general",
            QuicMode::Random => "quic-random",
            QuicMode::Shuffle => "quic-shuffle",
            QuicMode::Block => "quic-block",
            QuicMode::Pass => "quic-none",
            QuicMode::Adaptive => "quic-adaptive",
            QuicMode::Split => "quic-split",
        }
    }
}

#[derive(Clone, Debug)]
pub struct QuicCfg {
    pub mode: QuicMode,
    pub fake: Vec<u8>,
    pub hello: Vec<u8>,
    pub scid: Vec<u8>,
    pub repeats: usize,
}

impl QuicCfg {
    pub fn new(mode: QuicMode, fake: Vec<u8>) -> Option<QuicCfg> {
        let (i, hello) = quic::hello_of(&fake)?;
        Some(QuicCfg { mode, fake, hello, scid: i.scid, repeats: 11 })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Collect,
    Done,
    Blocked,
}

pub(super) struct QuicFlow<A> {
    held: Vec<(Vec<u8>, A)>,
    chunks: Vec<(u64, Vec<u8>)>,
    since: Instant,
    last: Instant,
    state: State,
}

pub fn shuffle(d: &[u8], cut: Option<u64>, rnd: &mut dyn FnMut(&mut [u8])) -> Option<Vec<u8>> {
    let i = quic::open_initial(d)?;
    let chunks = quic::crypto_chunks(&i.frames)?;
    if chunks.is_empty() {
        return None;
    }
    let budget = i.size.checked_sub(quic::header_len(&i.dcid, &i.scid, &i.token) + 2 + 4 + 16)?;
    let frame_len = |pieces: &[(u64, Vec<u8>)]| -> usize {
        pieces
            .iter()
            .map(|(o, b)| {
                let mut h = Vec::new();
                quic::crypto_frame(&mut h, *o, b);
                h.len()
            })
            .sum()
    };
    let cut_up = |size: usize| -> Vec<(u64, Vec<u8>)> {
        let mut out = Vec::new();
        for (off, data) in &chunks {
            let mut at = 0usize;
            while at < data.len() {
                let mut end = (at + size).min(data.len());
                if let Some(c) = cut {
                    let local = c.checked_sub(*off).map(|x| x as usize);
                    if let Some(l) = local.filter(|l| *l > at && *l < end) {
                        end = l;
                    }
                }
                out.push((*off + at as u64, data[at..end].to_vec()));
                at = end;
            }
        }
        out
    };
    let mut pieces = PIECES.iter().map(|s| cut_up(*s)).find(|p| p.len() > 1 && frame_len(p) <= budget)?;
    for k in (1..pieces.len()).rev() {
        let j = pick(k + 1, rnd);
        pieces.swap(k, j);
    }
    if pieces[0].0 == 0 && pieces.len() > 1 {
        let last = pieces.len() - 1;
        pieces.swap(0, last);
    }
    let mut frames = Vec::new();
    for (o, b) in &pieces {
        quic::crypto_frame(&mut frames, *o, b);
    }
    let mut out = quic::seal_initial(&i.dcid, &i.scid, &i.token, i.pn, &frames, i.size);
    if out.len() != i.size {
        return None;
    }
    out.extend_from_slice(&d[i.size..]);
    Some(out)
}

const SPLIT_PIECE: usize = 1100;

pub fn split_datagrams(first: &[u8], stream: &[u8], cut: usize) -> Option<Vec<Vec<u8>>> {
    let i = quic::open_initial(first)?;
    if cut == 0 || cut >= stream.len() {
        return None;
    }
    let mut pieces: Vec<(usize, usize)> = vec![(0, cut)];
    let mut at = cut;
    while at < stream.len() {
        let end = (at + SPLIT_PIECE).min(stream.len());
        pieces.push((at, end));
        at = end;
    }
    let n = pieces.len() as u64;
    let base = if i.pn + 1 >= n { i.pn + 1 - n } else { i.pn };
    let size = i.size.max(quic::MIN_DATAGRAM);
    Some(
        pieces
            .iter()
            .enumerate()
            .map(|(k, (from, to))| {
                let mut frames = Vec::new();
                quic::crypto_frame(&mut frames, *from as u64, &stream[*from..*to]);
                quic::seal_initial(&i.dcid, &i.scid, &i.token, base + k as u64, &frames, size)
            })
            .collect(),
    )
}

impl<A: Clone> Engine<A> {
    fn other_udp(&mut self, raw: &[u8], meta: A, now: Instant) -> Action<A> {
        let Some(u) = packet::parse_udp(raw) else { return Action::Pass };
        let data = u.payload();
        let own = self.lists.own;
        let (fake, repeats) = if own.voice && profiles::in_voice(u.dport) && profiles::is_discord_or_stun(data) && !self.cfg.fake_voice.is_empty() {
            self.stats.voice += 1;
            (self.cfg.fake_voice.clone(), profiles::VOICE_REPEATS)
        } else if own.game_udp
            && !self.cfg.fake_game_udp.is_empty()
            && self.lists.game_udp.contains(u.dport)
            && self.lists.ipset.contains(&u.dst)
            && !self.lists.ip_excludes.contains(&u.dst)
        {
            if self.game_seen.len() >= super::MAX_FLOWS {
                self.game_seen.clear();
            }
            let seen = self.game_seen.entry((u.src, u.sport, u.dst, u.dport)).or_insert((0, now));
            seen.1 = now;
            if seen.0 >= profiles::GAME_UDP_PACKETS {
                return Action::Pass;
            }
            seen.0 += 1;
            self.stats.game_udp += 1;
            (self.cfg.fake_game_udp.clone(), profiles::GAME_UDP_REPEATS)
        } else if self.udp_auto() && self.lists.game_udp.contains(u.dport) && !self.lists.ip_excludes.contains(&u.dst) {
            return self.auto_udp(raw, meta, now);
        } else {
            return Action::Pass;
        };
        let f = packet::udp_like(&u, &fake);
        let mut out: Vec<(Vec<u8>, A)> = (0..repeats).map(|_| (f.clone(), meta.clone())).collect();
        out.push((packet::fixed(raw), meta));
        Action::Send(out)
    }

    pub(super) fn handle_udp(&mut self, raw: &[u8], meta: A, now: Instant) -> Action<A> {
        let Some(u) = packet::parse_udp(raw) else { return Action::Pass };
        let data = u.payload();
        if self.quic.is_none() || u.dport != 443 || !quic::is_initial(data) {
            return self.other_udp(raw, meta, now);
        }
        let key = (u.src, u.sport, u.dst, u.dport);
        let state = self.qflows.get_mut(&key).map(|f| {
            f.last = now;
            f.state
        });
        match state {
            Some(State::Done) => {
                let ack_only = || quic::open_initial(data).is_some_and(|i| quic::crypto_chunks(&i.frames).is_none_or(|c| c.is_empty()));
                let moved = self.pending.get(&key).and_then(|p| p.dcid.as_deref()).is_some_and(|orig| quic::long_dcid(data) != Some(orig));
                if self.forward && self.pending.contains_key(&key) && (moved || ack_only()) {
                    if let Some(p) = self.pending.remove(&key) {
                        self.settle(p, Outcome::Answered);
                    }
                }
                return Action::Pass;
            }
            Some(State::Blocked) => return Action::Drop,
            _ => {}
        }
        let chunks = quic::open_initial(data).and_then(|i| quic::crypto_chunks(&i.frames)).unwrap_or_default();
        if !self.qflows.contains_key(&key) {
            if chunks.is_empty() {
                return Action::Pass;
            }
            if self.qflows.len() >= super::MAX_FLOWS {
                self.qflows.retain(|_, f| f.state == State::Collect);
            }
            self.qflows.insert(key, QuicFlow { held: Vec::new(), chunks: Vec::new(), since: now, last: now, state: State::Collect });
        }
        let f = self.qflows.get_mut(&key).expect("flow inserted");
        f.held.push((packet::fixed(raw), meta));
        f.chunks.extend(chunks);
        let stream = quic::assemble(&f.chunks);
        if let Some(need) = quic::hello_len(&stream).filter(|n| stream.len() >= *n) {
            f.state = State::Done;
            let held = std::mem::take(&mut f.held);
            f.chunks.clear();
            return self.decide_quic(key, &stream[..need], held, now);
        }
        if f.held.len() >= MAX_HELD {
            f.state = State::Done;
            f.chunks.clear();
            self.stats.flushed += 1;
            return Action::Send(std::mem::take(&mut f.held));
        }
        Action::Hold
    }

    fn decide_quic(&mut self, key: Key, hello: &[u8], held: Vec<(Vec<u8>, A)>, now: Instant) -> Action<A> {
        let single = held.len() == 1;
        let dcid = held.first().and_then(|(r, _)| packet::parse_udp(r)).and_then(|u| quic::long_dcid(u.payload()).map(<[u8]>::to_vec));
        let host = quic::sni(hello);
        let owner = self.owner_of(key.0, key.1, key.2, key.3, now);
        let (wanted, why, _) = self.verdict(host.as_deref(), &key.2, key.3, true, &owner);
        let who = Self::who(&host, &owner);
        let Some(cfg) = self.quic.clone() else { return Action::Send(held) };
        let mode = if cfg.mode == QuicMode::Adaptive { self.quic_choose(host.as_deref()) } else { cfg.mode };
        let tech = mode.tech();
        if !wanted {
            self.stats.skipped += 1;
            self.record(&host, &owner, false, why, None);
            self.note(format!("пропущен QUIC {who}: {why}"));
            return if single { Action::Pass } else { Action::Send(held) };
        }
        let id = self.record(&host, &owner, true, why, Some(tech));
        self.note(format!("обработан QUIC {who}: {why}, приём {tech}"));
        self.stats.handled += 1;
        self.stats.by_tech.entry(tech).or_default().used += 1;
        let first_raw = held[0].0.clone();
        let Some(first) = packet::parse_udp(&first_raw) else { return Action::Send(held) };
        let meta = held[0].1.clone();
        let mut out: Vec<(Vec<u8>, A)> = Vec::new();
        match mode {
            QuicMode::Pass => out.extend(held),
            QuicMode::Block => {
                if let Some(f) = self.qflows.get_mut(&key) {
                    f.state = State::Blocked;
                }
                return Action::Drop;
            }
            QuicMode::Fake | QuicMode::Adaptive => {
                let fake = packet::udp_like(&first, &cfg.fake);
                for _ in 0..cfg.repeats {
                    out.push((fake.clone(), meta.clone()));
                }
                out.extend(held);
            }
            QuicMode::Random => {
                let n = 6 + pick(7, &mut *self.rnd);
                for _ in 0..n {
                    let name = SNI_POOL[pick(SNI_POOL.len(), &mut *self.rnd)];
                    let Some(h) = quic::with_sni(&cfg.hello, name) else { continue };
                    let mut dcid = [0u8; 8];
                    (self.rnd)(&mut dcid);
                    let mut frames = Vec::new();
                    quic::crypto_frame(&mut frames, 0, &h);
                    let size = quic::MIN_DATAGRAM + pick(200, &mut *self.rnd);
                    let pkt = quic::seal_initial(&dcid, &cfg.scid, &[], 0, &frames, size);
                    out.push((packet::udp_like(&first, &pkt), meta.clone()));
                }
                out.extend(held);
            }
            QuicMode::Split => {
                let cut = tls::sni_range(&quic::as_record(hello)).map(|(at, len)| at - 5 + len / 2);
                match cut.and_then(|c| split_datagrams(first.payload(), hello, c)) {
                    Some(parts) => out.extend(parts.iter().map(|p| (packet::udp_like(&first, p), meta.clone()))),
                    None => out.extend(held),
                }
            }
            QuicMode::Shuffle => {
                let cut = tls::sni_range(&quic::as_record(hello)).map(|(at, len)| (at - 5 + len / 2) as u64);
                for (raw, m) in held {
                    let rebuilt = packet::parse_udp(&raw).and_then(|u| shuffle(u.payload(), cut, &mut *self.rnd).map(|p| packet::udp_like(&u, &p)));
                    out.push((rebuilt.unwrap_or(raw), m));
                }
            }
        }
        self.pending.insert(key, Pending { host, tech, since: now, id, ack0: None, dcid, bg: false, node: None, fname: None });
        Action::Send(out)
    }

    fn quic_choose(&self, host: Option<&str>) -> QuicMode {
        host.and_then(|h| self.qmemory.get(h)).map(|m| ROTATION[m.idx.min(ROTATION.len() - 1)]).unwrap_or(QuicMode::Fake)
    }

    pub(super) fn quic_adaptive(&self) -> bool {
        self.quic.as_ref().is_some_and(|q| q.mode == QuicMode::Adaptive)
    }

    pub(super) fn quic_learn(&mut self, p: &Pending, outcome: Outcome) {
        let Some(h) = p.host.clone() else { return };
        if !self.quic_adaptive() {
            return;
        }
        let cur = ROTATION.iter().position(|m| m.tech() == p.tech);
        let m = self.qmemory.entry_or(&h, Memory::default);
        let here = cur == Some(m.idx.min(ROTATION.len() - 1));
        if outcome == Outcome::Answered {
            m.ok += 1;
            if here {
                m.streak = 0;
            }
            return;
        }
        m.fails += 1;
        if here {
            m.streak += 1;
            if m.streak >= SWITCH_AFTER && m.idx + 1 < ROTATION.len() {
                m.idx += 1;
                m.streak = 0;
            }
        }
    }

    pub(super) fn quic_memory_lines(&self) -> Vec<(String, String)> {
        if !self.quic_adaptive() {
            return Vec::new();
        }
        self.qmemory.by_age().into_iter().map(|(h, m)| (h.clone(), format!("{}\t{}\t{}", ROTATION[m.idx.min(ROTATION.len() - 1)].tech(), m.ok, m.fails))).collect()
    }

    pub(super) fn quic_load(&mut self, host: &str, tech: &str, ok: u32, fails: u32) -> bool {
        if !self.quic_adaptive() {
            return false;
        }
        let Some(idx) = ROTATION.iter().position(|m| m.tech() == tech) else { return false };
        self.qmemory.insert(host, Memory { idx, ok, fails, ..Default::default() });
        true
    }

    pub fn quic_memory_of(&self, host: &str) -> Option<(&'static str, Memory)> {
        self.qmemory.get(host).map(|m| (ROTATION[m.idx.min(ROTATION.len() - 1)].tech(), *m))
    }

    pub(super) fn observe_udp(&mut self, raw: &[u8]) {
        let Some(u) = packet::parse_udp(raw) else { return };
        if self.observe_game_udp(&u) {
            return;
        }
        if !quic::is_long_header(u.payload()) {
            return;
        }
        let key = (u.dst, u.dport, u.src, u.sport);
        if let Some(p) = self.pending.remove(&key) {
            self.settle(p, Outcome::Answered);
        }
    }

    pub(super) fn expire_quic(&mut self, now: Instant) -> Vec<(Vec<u8>, A)> {
        let mut out = Vec::new();
        for f in self.qflows.values_mut() {
            if f.state == State::Collect && now.duration_since(f.since) >= REASM_WAIT {
                f.state = State::Done;
                f.chunks.clear();
                self.stats.flushed += 1;
                out.append(&mut f.held);
            }
        }
        self.qflows.retain(|_, f| now.duration_since(f.last) < IDLE);
        self.game_seen.retain(|_, s| now.duration_since(s.1) < IDLE);
        out
    }

    pub(super) fn drain_quic(&mut self) -> Vec<(Vec<u8>, A)> {
        self.qflows.drain().flat_map(|(_, f)| f.held).collect()
    }
}
