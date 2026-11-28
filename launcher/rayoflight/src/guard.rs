use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

pub const POLL: Duration = Duration::from_millis(500);
pub const STUCK_POLLS: u32 = 10;
pub const IDLE_POLLS: u32 = 40;
pub const GAP: Duration = Duration::from_secs(3);
pub const GRACE: Duration = Duration::from_secs(5);
pub const CALM: Duration = Duration::from_secs(2);
pub const SLOW: Duration = Duration::from_secs(60);
pub const BURST: usize = 5;
pub const BURST_WINDOW: Duration = Duration::from_secs(600);

static FAULT: AtomicBool = AtomicBool::new(false);

pub fn fault() {
    FAULT.store(true, Ordering::SeqCst);
}

pub fn take_fault() -> bool {
    FAULT.swap(false, Ordering::SeqCst)
}

pub const TEST: bool = option_env!("HZ_CHANNEL").is_some();
pub const ORDER_FILE: &str = "guard-test.txt";

static STUCK: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Order {
    Stuck,
    Ticker,
    Panic,
}

pub fn parse_order(text: &str) -> Option<Order> {
    let t = text.to_lowercase();
    [("stuck", Order::Stuck), ("ticker", Order::Ticker), ("panic", Order::Panic)].into_iter().find(|(k, _)| t.contains(k)).map(|(_, o)| o)
}

pub fn take_order(dir: &std::path::Path) -> Option<Order> {
    let path = dir.join(ORDER_FILE);
    let raw = std::fs::read(&path).ok()?;
    let _ = std::fs::remove_file(&path);
    parse_order(&String::from_utf8_lossy(&raw).replace('\0', ""))
}

pub fn order_stuck() {
    STUCK.store(true, Ordering::SeqCst);
}

pub fn take_stuck() -> bool {
    STUCK.swap(false, Ordering::SeqCst)
}

#[derive(Default)]
pub struct Pulse {
    seq: AtomicU64,
    ticks: AtomicU64,
}

impl Pulse {
    pub fn enter(&self) {
        self.seq.fetch_or(1, Ordering::SeqCst);
    }

    pub fn leave(&self) {
        let v = self.seq.load(Ordering::SeqCst);
        self.seq.store((v | 1).wrapping_add(1), Ordering::SeqCst);
    }

    pub fn beat(&self) {
        self.ticks.fetch_add(1, Ordering::SeqCst);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    Stuck,
    Ticker,
    Fault,
}

impl Why {
    pub fn text(self) -> &'static str {
        match self {
            Why::Stuck => "перехват завис и не отпускает пакеты",
            Why::Ticker => "служебный поток движка не отвечает",
            Why::Fault => "сбой в одном из потоков движка",
        }
    }
}

#[derive(Default)]
pub struct Watch {
    seq: u64,
    ticks: u64,
    stuck: u32,
    idle: u32,
}

impl Watch {
    pub fn poll(&mut self, p: &Pulse, gap: Duration, fault: bool) -> Option<Why> {
        let (seq, ticks) = (p.seq.load(Ordering::SeqCst), p.ticks.load(Ordering::SeqCst));
        let slept = gap > GAP;
        self.stuck = if seq & 1 == 1 && seq == self.seq && !slept { self.stuck + 1 } else { 0 };
        self.idle = if ticks == self.ticks && !slept { self.idle + 1 } else { 0 };
        self.seq = seq;
        self.ticks = ticks;
        if fault {
            return Some(Why::Fault);
        }
        if self.stuck >= STUCK_POLLS {
            return Some(Why::Stuck);
        }
        (self.idle >= IDLE_POLLS).then_some(Why::Ticker)
    }
}

#[derive(Default)]
pub struct Restarts {
    at: Vec<Instant>,
    pub total: u32,
}

impl Restarts {
    pub fn next(&mut self, now: Instant) -> Duration {
        self.at.retain(|t| now.duration_since(*t) < BURST_WINDOW);
        self.at.push(now);
        self.total += 1;
        if self.at.len() > BURST { SLOW } else { CALM }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn polls(w: &mut Watch, p: &Pulse, n: u32) -> Option<Why> {
        (0..n).find_map(|_| w.poll(p, POLL, false))
    }

    #[test]
    fn busy_loop_that_keeps_moving_is_not_stuck() {
        let (p, mut w) = (Pulse::default(), Watch::default());
        for _ in 0..100 {
            p.enter();
            p.beat();
            assert_eq!(w.poll(&p, POLL, false), None);
            p.leave();
        }
    }

    #[test]
    fn waiting_for_packets_is_not_stuck() {
        let (p, mut w) = (Pulse::default(), Watch::default());
        p.enter();
        p.leave();
        for _ in 0..100 {
            p.beat();
            assert_eq!(w.poll(&p, POLL, false), None);
        }
    }

    #[test]
    fn packet_held_too_long_is_stuck() {
        let (p, mut w) = (Pulse::default(), Watch::default());
        p.enter();
        for _ in 0..STUCK_POLLS {
            p.beat();
            assert_eq!(w.poll(&p, POLL, false), None);
        }
        p.beat();
        assert_eq!(w.poll(&p, POLL, false), Some(Why::Stuck));
    }

    #[test]
    fn silent_ticker_is_noticed() {
        let (p, mut w) = (Pulse::default(), Watch::default());
        p.beat();
        assert_eq!(polls(&mut w, &p, IDLE_POLLS), None);
        assert_eq!(w.poll(&p, POLL, false), Some(Why::Ticker));
    }

    #[test]
    fn sleep_of_the_computer_resets_the_count() {
        let (p, mut w) = (Pulse::default(), Watch::default());
        p.enter();
        assert_eq!(polls(&mut w, &p, STUCK_POLLS), None);
        assert_eq!(w.poll(&p, Duration::from_secs(3600), false), None);
        assert_eq!(polls(&mut w, &p, STUCK_POLLS - 1), None);
    }

    #[test]
    fn fault_is_reported_once() {
        let (p, mut w) = (Pulse::default(), Watch::default());
        fault();
        assert_eq!(w.poll(&p, POLL, take_fault()), Some(Why::Fault));
        assert_eq!(w.poll(&p, POLL, take_fault()), None);
    }

    #[test]
    fn test_order_is_read_once_in_any_encoding() {
        let dir = std::env::temp_dir().join(format!("hz-guard-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let utf16: Vec<u8> = "\u{feff}Ticker\r\n".encode_utf16().flat_map(|c| c.to_le_bytes()).collect();
        std::fs::write(dir.join(ORDER_FILE), utf16).unwrap();
        assert_eq!(take_order(&dir), Some(Order::Ticker));
        assert_eq!(take_order(&dir), None);
        assert_eq!(parse_order("\u{feff}stuck\r\n"), Some(Order::Stuck));
        assert_eq!(parse_order("panic"), Some(Order::Panic));
        assert_eq!(parse_order("что-то"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn many_restarts_in_a_row_slow_down() {
        let (mut r, now) = (Restarts::default(), Instant::now());
        for i in 0..BURST {
            assert_eq!(r.next(now + Duration::from_secs(i as u64)), CALM);
        }
        assert_eq!(r.next(now + Duration::from_secs(10)), SLOW);
        assert_eq!(r.next(now + BURST_WINDOW + Duration::from_secs(11)), CALM);
        assert_eq!(r.total, BURST as u32 + 2);
    }
}
