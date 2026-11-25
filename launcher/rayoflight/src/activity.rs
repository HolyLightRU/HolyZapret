use std::collections::VecDeque;
use std::time::{Duration, Instant};

pub const MINUTES: usize = 30;
const MINUTE: Duration = Duration::from_secs(60);

pub type Counts = (u64, u64, u64);

pub struct Activity {
    prev: Counts,
    now: Counts,
    buckets: VecDeque<Counts>,
    at: Instant,
    cpu_prev: Option<(Instant, u64)>,
    cpu: f64,
    mem: u64,
    cores: f64,
}

fn minus(a: Counts, b: Counts) -> Counts {
    (a.0.saturating_sub(b.0), a.1.saturating_sub(b.1), a.2.saturating_sub(b.2))
}

impl Activity {
    pub fn new(now: Instant) -> Self {
        let cores = std::thread::available_parallelism().map(|n| n.get() as f64).unwrap_or(1.0);
        Activity { prev: (0, 0, 0), now: (0, 0, 0), buckets: VecDeque::new(), at: now, cpu_prev: None, cpu: 0.0, mem: 0, cores }
    }

    pub fn update(&mut self, totals: Counts, now: Instant) {
        self.now = totals;
        while now.duration_since(self.at) >= MINUTE {
            self.buckets.push_back(minus(totals, self.prev));
            self.prev = totals;
            self.at += MINUTE;
            if self.buckets.len() > MINUTES {
                self.buckets.pop_front();
            }
        }
    }

    pub fn load(&mut self, cpu_time: u64, mem: u64, now: Instant) {
        if let Some((t, c)) = self.cpu_prev {
            let wall = now.duration_since(t).as_nanos() as f64 / 100.0;
            if wall > 0.0 {
                self.cpu = (cpu_time.saturating_sub(c) as f64 / wall / self.cores * 100.0).clamp(0.0, 100.0);
            }
        }
        self.cpu_prev = Some((now, cpu_time));
        self.mem = mem;
    }

    pub fn json(&self) -> String {
        let mut rows: Vec<String> = self.buckets.iter().map(|b| format!("[{},{},{}]", b.0, b.1, b.2)).collect();
        let cur = minus(self.now, self.prev);
        rows.push(format!("[{},{},{}]", cur.0, cur.1, cur.2));
        format!("\"history\":[{}],\"cpu\":{:.2},\"mem\":{},", rows.join(","), self.cpu, self.mem)
    }
}

pub fn with_extra(json: &str, extra: &str) -> String {
    match json.strip_prefix('{') {
        Some(rest) => format!("{{{extra}{rest}"),
        None => json.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets_by_minute_and_keeps_thirty() {
        let t0 = Instant::now();
        let mut a = Activity::new(t0);
        a.update((5, 4, 1), t0 + Duration::from_secs(30));
        assert!(a.json().starts_with("\"history\":[[5,4,1]]"));
        a.update((9, 7, 2), t0 + Duration::from_secs(61));
        assert!(a.json().starts_with("\"history\":[[9,7,2],[0,0,0]]"));
        a.update((12, 9, 3), t0 + Duration::from_secs(185));
        assert!(a.json().starts_with("\"history\":[[9,7,2],[3,2,1],[0,0,0],[0,0,0]]"), "{}", a.json());
        for m in 4..80 {
            a.update((12 + m, 9, 3), t0 + Duration::from_secs(60 * m + 1));
        }
        assert_eq!(a.buckets.len(), MINUTES);
    }

    #[test]
    fn cpu_share_and_extra_insert() {
        let t0 = Instant::now();
        let mut a = Activity::new(t0);
        a.cores = 4.0;
        a.load(1_000_000, 8 << 20, t0);
        a.load(1_000_000 + 4_000_000, 9 << 20, t0 + Duration::from_secs(2));
        assert!((a.cpu - 5.0).abs() < 0.01, "{}", a.cpu);
        assert!(a.json().contains("\"cpu\":5.00,\"mem\":9437184,"));
        assert_eq!(with_extra("{\"running\":true}", "\"x\":1,"), "{\"x\":1,\"running\":true}");
    }
}
