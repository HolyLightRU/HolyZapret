use super::{Action, Engine, IDLE, Key, MAX_FLOWS, MAX_MEMORY, Memory, Mode, Outcome, Pending, SILENT};
use crate::packet::{self, Udp};
use crate::procs::{AppRule, Owner};
use crate::profiles;
use std::collections::HashMap;
use std::time::{Duration, Instant};

pub const UDP_ROTATION: [&str; 4] = ["udp-none", "udp-fake", "udp-random", "udp-quic"];
pub const UDP_SWITCH_AFTER: u32 = 3;
pub const UDP_KEY: &str = "udp:";
pub const UDP_WHY: &str = "игра: самоподстройка UDP";
pub const UDP_WINDOW: Duration = Duration::from_secs(60);
const NOISY_WINDOW: Duration = Duration::from_secs(60);
const NOISY_FLOWS: u32 = 120;
const NOISY_HOLD: Duration = Duration::from_secs(600);
const PENDING_PER_APP: usize = 2;
const RANDOM_FAKES: usize = 6;
const QUIC_FAKES: usize = 6;

pub struct UFlow {
    tech: &'static str,
    sent: u32,
    last: Instant,
}

#[derive(Default)]
pub struct Noise {
    since: Option<Instant>,
    flows: u32,
    until: Option<Instant>,
}

pub fn udp_key(app: Option<&str>) -> String {
    format!("{UDP_KEY}{}", app.unwrap_or("неизвестная программа"))
}

impl<A: Clone> Engine<A> {
    pub(super) fn udp_auto(&self) -> bool {
        self.lists.own.game_udp && !self.forward && matches!(self.mode, Mode::Adaptive(_))
    }

    fn udp_choose(&self, key: &str) -> &'static str {
        self.umemory.get(key).map(|m| UDP_ROTATION[m.idx % UDP_ROTATION.len()]).unwrap_or(UDP_ROTATION[0])
    }

    fn noisy(&mut self, key: &str, now: Instant) -> bool {
        if self.noise.len() >= MAX_MEMORY && !self.noise.contains_key(key) {
            self.noise.clear();
        }
        let n = self.noise.entry(key.to_string()).or_default();
        if n.until.is_some_and(|u| now < u) {
            return true;
        }
        if n.since.is_none_or(|s| now.duration_since(s) >= NOISY_WINDOW) {
            n.since = Some(now);
            n.flows = 0;
        }
        n.flows += 1;
        if n.flows > NOISY_FLOWS {
            n.until = Some(now + NOISY_HOLD);
            return true;
        }
        false
    }

    fn udp_owner(&mut self, local_port: u16, now: Instant) -> Option<Owner> {
        self.udp_of.as_mut().and_then(|f| f(local_port, now))
    }

    pub(super) fn auto_udp(&mut self, raw: &[u8], meta: A, now: Instant) -> Action<A> {
        let Some(u) = packet::parse_udp(raw) else { return Action::Pass };
        let key: Key = (u.src, u.sport, u.dst, u.dport);
        if let Some(f) = self.uflows.get_mut(&key) {
            f.last = now;
            if f.sent >= profiles::GAME_UDP_PACKETS {
                return Action::Pass;
            }
            f.sent += 1;
            let tech = f.tech;
            return self.udp_send(&u, raw, meta, tech);
        }
        if self.uflows.len() >= MAX_FLOWS {
            self.uflows.retain(|_, f| now.duration_since(f.last) < IDLE);
            if self.uflows.len() >= MAX_FLOWS {
                self.uflows.clear();
            }
        }
        let owner = self.udp_owner(u.sport, now);
        let app = owner.as_ref().and_then(|o| o.name.clone());
        let host = udp_key(app.as_deref());
        let skip = matches!(self.lists.apps.rule(app.as_deref()), AppRule::Skip);
        if skip || self.noisy(&host, now) {
            self.uflows.insert(key, UFlow { tech: UDP_ROTATION[0], sent: profiles::GAME_UDP_PACKETS, last: now });
            return Action::Pass;
        }
        let tech = self.udp_choose(&host);
        self.uflows.insert(key, UFlow { tech, sent: 1, last: now });
        let waiting = self.upending.values().filter(|p| p.host.as_deref() == Some(host.as_str())).count();
        if waiting < PENDING_PER_APP && self.upending.len() < MAX_FLOWS {
            let id = self.record(&Some(host.clone()), &owner, true, UDP_WHY, Some(tech));
            self.stats.by_tech.entry(tech).or_default().used += 1;
            if tech != UDP_ROTATION[0] {
                self.stats.handled += 1;
            }
            self.upending.insert(key, Pending { host: Some(host), tech, since: now, id, ack0: None, dcid: None, bg: false });
        }
        if tech != UDP_ROTATION[0] {
            self.stats.game_udp += 1;
        }
        self.udp_send(&u, raw, meta, tech)
    }

    fn udp_send(&mut self, u: &Udp, raw: &[u8], meta: A, tech: &str) -> Action<A> {
        let game = self.cfg.fake_game_udp.clone();
        let fakes: Vec<Vec<u8>> = match tech {
            "udp-fake" if !game.is_empty() => vec![packet::udp_like(u, &game); profiles::GAME_UDP_REPEATS],
            "udp-random" => (0..RANDOM_FAKES)
                .map(|_| {
                    let mut n = [0u8; 1];
                    (self.rnd)(&mut n);
                    let mut b = vec![0u8; 32 + n[0] as usize];
                    (self.rnd)(&mut b);
                    packet::udp_like(u, &b)
                })
                .collect(),
            "udp-quic" => match self.quic.as_ref().map(|q| q.fake.clone()).filter(|f| !f.is_empty()) {
                Some(f) => vec![packet::udp_like(u, &f); QUIC_FAKES],
                None if !game.is_empty() => vec![packet::udp_like(u, &game); profiles::GAME_UDP_REPEATS],
                None => return Action::Pass,
            },
            _ => return Action::Pass,
        };
        let mut out: Vec<(Vec<u8>, A)> = fakes.into_iter().map(|f| (f, meta.clone())).collect();
        out.push((packet::fixed(raw), meta));
        Action::Send(out)
    }

    pub(super) fn observe_game_udp(&mut self, u: &Udp) -> bool {
        let key: Key = (u.dst, u.dport, u.src, u.sport);
        match self.upending.remove(&key) {
            Some(p) => {
                self.settle(p, Outcome::Answered);
                true
            }
            None => false,
        }
    }

    pub(super) fn expire_udp(&mut self, now: Instant) {
        let late: Vec<Key> = self.upending.iter().filter(|(_, p)| now.duration_since(p.since) >= SILENT).map(|(k, _)| *k).collect();
        for k in late {
            if let Some(p) = self.upending.remove(&k) {
                self.settle(p, Outcome::Silent);
            }
        }
        self.uflows.retain(|_, f| now.duration_since(f.last) < IDLE);
    }

    pub(super) fn udp_learn(&mut self, p: &Pending, outcome: Outcome) {
        let Some(h) = p.host.clone() else { return };
        if self.umemory.len() >= MAX_MEMORY && !self.umemory.contains_key(&h) {
            return;
        }
        let n = UDP_ROTATION.len();
        let cur = UDP_ROTATION.iter().position(|t| *t == p.tech);
        if outcome == Outcome::Answered {
            let last = self.ulast_ok.entry(h.clone()).or_insert(p.since);
            if p.since > *last {
                *last = p.since;
            }
        }
        let answered_lately = self.ulast_ok.get(&h).is_some_and(|t| p.since.saturating_duration_since(*t) < UDP_WINDOW);
        let m = self.umemory.entry(h).or_default();
        let here = cur == Some(m.idx % n);
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
            if m.streak >= UDP_SWITCH_AFTER && !answered_lately {
                m.idx = (m.idx + 1) % n;
                m.streak = 0;
            }
        }
    }

    pub(super) fn udp_memory_lines(&self) -> Vec<(String, String)> {
        self.umemory.iter().map(|(h, m)| (h.clone(), format!("{}\t{}\t{}", UDP_ROTATION[m.idx % UDP_ROTATION.len()], m.ok, m.fails))).collect()
    }

    pub(super) fn udp_load(&mut self, host: &str, tech: &str, ok: u32, fails: u32) -> bool {
        if self.umemory.len() >= MAX_MEMORY {
            return false;
        }
        let Some(idx) = UDP_ROTATION.iter().position(|t| *t == tech) else { return false };
        self.umemory.insert(host.to_string(), Memory { idx, ok, fails, streak: 0 });
        true
    }

    pub fn udp_memory_of(&self, key: &str) -> Option<(&'static str, Memory)> {
        self.umemory.get(key).map(|m| (UDP_ROTATION[m.idx % UDP_ROTATION.len()], *m))
    }
}

pub type UdpMemory = HashMap<String, Memory>;
