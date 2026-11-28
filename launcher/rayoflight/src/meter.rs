use windows_sys::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};

pub const EDGES_US: [u64; 4] = [100, 500, 1000, 5000];

#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lat {
    pub n: u64,
    pub sum: u64,
    pub max: u64,
    pub buckets: [u64; 5],
}

fn ms(us: u64) -> String {
    format!("{:.2}", us as f64 / 1000.0).replace('.', ",")
}

impl Lat {
    pub fn add(&mut self, us: u64) {
        self.n += 1;
        self.sum += us;
        self.max = self.max.max(us);
        let i = EDGES_US.iter().position(|e| us < *e).unwrap_or(EDGES_US.len());
        self.buckets[i] += 1;
    }

    pub fn line(&self, label: &str) -> Option<String> {
        if self.n == 0 {
            return None;
        }
        let need = self.n - self.n / 100;
        let mut seen = 0;
        let edge = EDGES_US.iter().zip(self.buckets.iter()).find_map(|(e, b)| {
            seen += b;
            (seen >= need).then_some(*e)
        });
        let most = match edge {
            Some(e) => format!("99% быстрее {} мс", ms(e)),
            None => format!("дольше {} мс: {}", ms(EDGES_US[EDGES_US.len() - 1]), self.buckets[EDGES_US.len()]),
        };
        Some(format!("задержка {label}: пакетов {}, в среднем {} мс, {most}, худшая {} мс", self.n, ms(self.sum / self.n), ms(self.max)))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Tcp,
    Udp,
    Handled,
}

pub const SLOW_US: u64 = 5000;
const SLACK_US: u64 = 200;

#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slow {
    pub late: u64,
    pub late_max: u64,
    pub busy: u64,
    pub busy_max: u64,
    pub lock_max: u64,
    pub handle_max: u64,
    pub send_max: u64,
}

impl Slow {
    pub fn add(&mut self, wait: u64, idle: u64, lock: u64, handle: u64, send: u64) {
        self.lock_max = self.lock_max.max(lock);
        self.handle_max = self.handle_max.max(handle);
        self.send_max = self.send_max.max(send);
        if wait < SLOW_US {
            return;
        }
        if idle + SLACK_US >= wait {
            self.late += 1;
            self.late_max = self.late_max.max(wait);
        } else {
            self.busy += 1;
            self.busy_max = self.busy_max.max(wait);
        }
    }

    pub fn line(&self) -> Option<String> {
        if self.late + self.busy == 0 && self.lock_max < SLOW_US && self.handle_max < SLOW_US && self.send_max < SLOW_US {
            return None;
        }
        Some(format!(
            "медленные пакеты: пришли к свободному движку с опозданием {} (до {} мс), ждали занятый движок {} (до {} мс); дольше всего ждали другие потоки движка {} мс, разбор {} мс, отправка {} мс",
            self.late,
            ms(self.late_max),
            self.busy,
            ms(self.busy_max),
            ms(self.lock_max),
            ms(self.handle_max),
            ms(self.send_max)
        ))
    }
}

#[derive(Default)]
pub struct Meter {
    tcp: Lat,
    udp: Lat,
    handled: Lat,
    slow: Slow,
}

impl Meter {
    pub fn add(&mut self, kind: Kind, us: u64) {
        match kind {
            Kind::Tcp => self.tcp.add(us),
            Kind::Udp => self.udp.add(us),
            Kind::Handled => self.handled.add(us),
        }
    }

    pub fn stages(&mut self, wait: u64, idle: u64, lock: u64, handle: u64, send: u64) {
        self.slow.add(wait, idle, lock, handle, send);
    }

    pub fn take_lines(&mut self) -> Vec<String> {
        let m = std::mem::take(self);
        let mut out: Vec<String> = [(m.tcp, "TCP без изменений"), (m.udp, "UDP без изменений"), (m.handled, "с обходом")].iter().filter_map(|(l, name)| l.line(name)).collect();
        out.extend(m.slow.line());
        out
    }
}

pub fn is_udp(raw: &[u8]) -> bool {
    match raw.first().map(|b| b >> 4) {
        Some(4) => raw.get(9) == Some(&17),
        Some(6) => raw.get(6) == Some(&17),
        _ => false,
    }
}

#[derive(Clone, Copy)]
pub struct Clock {
    freq: i64,
}

impl Clock {
    pub fn new() -> Clock {
        let mut freq = 0i64;
        unsafe { QueryPerformanceFrequency(&mut freq) };
        Clock { freq: freq.max(1) }
    }

    pub fn since_us(&self, stamp: i64) -> Option<u64> {
        let mut now = 0i64;
        unsafe { QueryPerformanceCounter(&mut now) };
        let d = now.checked_sub(stamp).filter(|d| stamp > 0 && *d >= 0)?;
        Some((d as u128 * 1_000_000 / self.freq as u128) as u64)
    }
}

impl Default for Clock {
    fn default() -> Self {
        Clock::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latency_line_shows_mean_tail_and_worst() {
        let mut l = Lat::default();
        assert_eq!(l.line("TCP"), None);
        for _ in 0..99 {
            l.add(40);
        }
        l.add(2300);
        assert_eq!(l.line("TCP").unwrap(), "задержка TCP: пакетов 100, в среднем 0,06 мс, 99% быстрее 0,10 мс, худшая 2,30 мс");
        let mut slow = Lat::default();
        for _ in 0..10 {
            slow.add(7000);
        }
        assert_eq!(slow.line("UDP").unwrap(), "задержка UDP: пакетов 10, в среднем 7,00 мс, дольше 5,00 мс: 10, худшая 7,00 мс");
    }

    #[test]
    fn meter_resets_after_report_and_tells_udp_from_tcp() {
        let mut m = Meter::default();
        m.add(Kind::Udp, 50);
        m.add(Kind::Handled, 300);
        let lines = m.take_lines();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("задержка UDP без изменений") && lines[1].starts_with("задержка с обходом"));
        assert!(m.take_lines().is_empty());
        let mut v4 = [0u8; 20];
        v4[0] = 0x45;
        v4[9] = 17;
        let mut v6 = [0u8; 40];
        v6[0] = 0x60;
        v6[6] = 6;
        assert!(is_udp(&v4) && !is_udp(&v6) && !is_udp(&[]));
    }

    #[test]
    fn slow_packets_are_split_by_who_made_them_wait() {
        let mut s = Slow::default();
        s.add(40, 900_000, 5, 30, 10);
        assert_eq!(s.line(), None);
        s.add(1_200_000, 1_500_000, 5, 60, 20);
        s.add(800_000, 3, 195_000, 70, 15);
        s.add(6_000, 100, 5, 9_000, 15);
        assert_eq!((s.late, s.late_max, s.busy, s.busy_max, s.lock_max, s.handle_max, s.send_max), (1, 1_200_000, 2, 800_000, 195_000, 9_000, 20));
        assert_eq!(
            s.line().unwrap(),
            "медленные пакеты: пришли к свободному движку с опозданием 1 (до 1200,00 мс), ждали занятый движок 2 (до 800,00 мс); дольше всего ждали другие потоки движка 195,00 мс, разбор 9,00 мс, отправка 0,02 мс"
        );
        let mut m = Meter::default();
        m.stages(7_000, 9_000, 1, 1, 1);
        assert_eq!(m.take_lines().len(), 1);
    }

    #[test]
    fn clock_measures_from_a_capture_stamp() {
        let c = Clock::new();
        let mut now = 0i64;
        unsafe { QueryPerformanceCounter(&mut now) };
        let us = c.since_us(now).unwrap();
        assert!(us < 1_000_000);
        assert_eq!(c.since_us(0), None);
        assert_eq!(c.since_us(i64::MAX), None);
    }
}
