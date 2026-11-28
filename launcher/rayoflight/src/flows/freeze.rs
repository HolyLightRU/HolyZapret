use super::{Engine, Key, Memory, Mode, Outcome};
use crate::desync;
use std::collections::HashMap;
use std::net::IpAddr;
use std::time::{Duration, Instant};

pub const FREEZE_LO: u32 = 12 * 1024;
pub const FREEZE_HI: u32 = 32 * 1024;
pub const FREEZE_STALL: Duration = Duration::from_secs(3);
pub const FREEZE_RESEND: Duration = Duration::from_secs(2);
pub const FREEZE_KEEP: Duration = Duration::from_secs(30 * 60);
pub const FREEZE_TO_WARP: u32 = 2;
const MAX_FROZEN: usize = 512;
const MAX_IPS: usize = 16;

#[derive(Clone, Debug)]
pub struct Watch {
    host: String,
    tech: &'static str,
    id: u64,
    dst: IpAddr,
    rcv: Option<u32>,
    since: Instant,
    next_seq: u32,
    resend_from: Option<Instant>,
}

#[derive(Clone, Debug, Default)]
pub struct Frozen {
    pub ips: Vec<IpAddr>,
    pub count: u32,
    pub last: Option<Instant>,
    pub checked: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fetch {
    Passed(usize),
    Stalled(usize),
    Short(usize),
    Failed,
}

pub const FETCH_WANT: usize = 64 * 1024;
pub const FETCH_PASS: usize = 32 * 1024;
pub const FETCH_LOW: usize = 6 * 1024;
pub const CHECK_WITHIN: Duration = Duration::from_secs(300);

pub fn fetch_verdict(got: usize, ended: bool) -> Fetch {
    match (got, ended) {
        (n, _) if n >= FETCH_PASS => Fetch::Passed(n),
        (n, true) => Fetch::Short(n),
        (n, false) if n >= FETCH_LOW => Fetch::Stalled(n),
        _ => Fetch::Failed,
    }
}

pub type FrozenMap = HashMap<String, Frozen>;

impl<A: Clone> Engine<A> {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn start_watch(&mut self, key: &Key, host: &Option<String>, tech: &'static str, id: u64, dst: IpAddr, next_seq: u32, now: Instant) {
        if self.forward {
            return;
        }
        let Some(h) = host else { return };
        if let Some(f) = self.flows.get_mut(key) {
            f.watch = Some(Watch { host: h.clone(), tech, id, dst, rcv: None, since: now, next_seq, resend_from: None });
        }
    }

    pub(super) fn watch_out(&mut self, key: &Key, seq: u32, ack: u32, len: u32, now: Instant) {
        let Some(f) = self.flows.get_mut(key) else { return };
        let (Some(isn), Some(w)) = (f.srv_isn, f.watch.as_mut()) else { return };
        let rcv = ack.wrapping_sub(isn.wrapping_add(1));
        if rcv > FREEZE_HI {
            f.watch = None;
            return;
        }
        let end = seq.wrapping_add(len);
        let resent = (end.wrapping_sub(w.next_seq) as i32) <= 0;
        if !resent {
            w.next_seq = end;
            w.resend_from = None;
        }
        if w.rcv != Some(rcv) {
            w.rcv = Some(rcv);
            w.since = now;
            w.resend_from = None;
            return;
        }
        if !resent {
            return;
        }
        let first = *w.resend_from.get_or_insert(now);
        if rcv < FREEZE_LO || now.saturating_duration_since(w.since) < FREEZE_STALL || now.saturating_duration_since(first) < FREEZE_RESEND {
            return;
        }
        if let Some(w) = f.watch.take() {
            self.froze(key, w, rcv, now);
        }
    }

    fn froze(&mut self, key: &Key, w: Watch, rcv: u32, now: Instant) {
        let answered = !self.pending.contains_key(key);
        self.stats.frozen += 1;
        if answered {
            self.stats.answered = self.stats.answered.saturating_sub(1);
        }
        let t = self.stats.by_tech.entry(w.tech).or_default();
        t.frozen += 1;
        if answered {
            t.answered = t.answered.saturating_sub(1);
        }
        if let Some(d) = self.recent.iter_mut().rev().find(|d| d.id == w.id) {
            d.outcome = Some(Outcome::Frozen);
        }
        if self.frozen.len() >= MAX_FROZEN && !self.frozen.contains_key(&w.host) {
            self.frozen.retain(|_, f| f.last.is_some_and(|l| now.saturating_duration_since(l) < FREEZE_KEEP));
        }
        let mut count = 0;
        if self.frozen.len() < MAX_FROZEN || self.frozen.contains_key(&w.host) {
            let f = self.frozen.entry(w.host.clone()).or_default();
            if f.last.is_some_and(|l| now.saturating_duration_since(l) >= FREEZE_KEEP) {
                *f = Frozen::default();
            }
            f.count += 1;
            f.last = Some(now);
            if !f.ips.contains(&w.dst) && f.ips.len() < MAX_IPS {
                f.ips.push(w.dst);
            }
            count = f.count;
        }
        self.flush_dns = true;
        let mut text = format!("заморозка: {} ({}) — сервер замолчал после {} КБ, приём {}", w.host, w.dst, rcv / 1024, w.tech);
        if count >= FREEZE_TO_WARP && self.warp_step(&w.host) {
            text.push_str(", дальше сайт пойдёт через WARP");
        } else if self.dns_alt {
            text.push_str(", попробую другой адрес сервера");
        }
        if self.notes.len() < 1000 {
            self.notes.push(text);
        }
    }

    pub fn freeze_targets(&mut self, now: Instant) -> Vec<String> {
        if self.forward {
            return Vec::new();
        }
        let pick = self.frozen.iter().find(|(_, f)| !f.checked && f.last.is_some_and(|l| now.saturating_duration_since(l) < CHECK_WITHIN)).map(|(h, _)| h.clone());
        if let Some(f) = pick.as_ref().and_then(|h| self.frozen.get_mut(h)) {
            f.checked = true;
        }
        pick.into_iter().collect()
    }

    pub fn freeze_verdict(&mut self, host: &str, v: Fetch) {
        let text = match v {
            Fetch::Passed(n) => {
                self.frozen.remove(host);
                format!("проверка заморозки: {host} — скачано {} КБ без обрыва, заморозка не подтвердилась", n / 1024)
            }
            Fetch::Stalled(n) => format!("проверка заморозки: {host} — загрузка оборвалась на {} КБ, заморозка подтверждена", n / 1024),
            Fetch::Short(n) => format!("проверка заморозки: {host} — главная страница всего {} КБ, по ней не проверить", n / 1024),
            Fetch::Failed => format!("проверка заморозки: {host} — соединиться для проверки не удалось"),
        };
        if self.notes.len() < 1000 {
            self.notes.push(text);
        }
    }

    pub fn is_frozen(&self, host: &str) -> bool {
        self.frozen.contains_key(host)
    }

    fn warp_step(&mut self, host: &str) -> bool {
        if self.warp.is_none() {
            return false;
        }
        let Mode::Adaptive(list) = &self.mode else { return false };
        let Some(idx) = list.iter().position(|t| t.name == desync::WARP.name) else { return false };
        let m = self.memory.entry_or(host, Memory::default);
        m.idx = idx;
        m.streak = 0;
        true
    }

    pub fn avoid(&self, host: &str, now: Instant) -> Vec<IpAddr> {
        self.frozen
            .get(host)
            .filter(|f| f.last.is_some_and(|l| now.saturating_duration_since(l) < FREEZE_KEEP))
            .map(|f| f.ips.clone())
            .unwrap_or_default()
    }

    pub fn take_flush(&mut self) -> bool {
        std::mem::take(&mut self.flush_dns)
    }
}
