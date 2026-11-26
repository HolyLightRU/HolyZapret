use crate::desync::{self, Config};
use crate::flows::{Action, Decision, Engine, Mode, QuicCfg, RECENT};
use crate::run::{self, Args, Log};
use crate::windivert::{self, Address, Handle, Shutter};
use crate::{sys, watch};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;

pub struct Gateway {
    handle: Arc<Handle>,
    redirects: Option<Arc<Handle>>,
    engine: Arc<Mutex<Engine<Address>>>,
    threads: Mutex<Vec<JoinHandle<()>>>,
    pub dropped: Arc<AtomicU64>,
    pub failed: Arc<AtomicU64>,
}

fn send_all(h: &Handle, out: Vec<(Vec<u8>, Address)>, failed: &AtomicU64) {
    for (p, a) in out {
        if !h.send_built(&p, &a) {
            failed.fetch_add(1, Ordering::Relaxed);
        }
    }
}

pub fn mode_for(experiment: &str) -> Mode {
    Mode::parse(experiment).unwrap_or(Mode::Fixed(desync::GENERAL))
}

pub fn quic_for(q: Option<QuicCfg>) -> Option<QuicCfg> {
    q
}

pub fn route_interface() -> Option<u32> {
    use windows_sys::Win32::NetworkManagement::IpHelper::GetBestInterface;
    let mut idx = 0u32;
    let dst = u32::from_ne_bytes([8, 8, 8, 8]);
    (unsafe { GetBestInterface(dst, &mut idx) } == 0).then_some(idx)
}

fn ensure_forwarding(log: &Log) {
    use std::os::windows::process::CommandExt;
    let Some(idx) = route_interface() else {
        log.line("шлюз: не найдена сетевая карта с выходом в интернет");
        return;
    };
    let out = std::process::Command::new("netsh")
        .args(["interface", "ipv4", "set", "interface", &idx.to_string(), "forwarding=enabled"])
        .creation_flags(0x0800_0000)
        .output();
    match out {
        Ok(o) if o.status.success() => log.line(&format!("шлюз: пересылка включена на сетевой карте {idx}")),
        Ok(o) => log.line(&format!("шлюз: пересылку не удалось включить: {}", String::from_utf8_lossy(&o.stdout).trim())),
        Err(e) => log.line(&format!("шлюз: netsh не запустился: {e}")),
    }
}

pub fn start(args: &Args, lib: &windivert::Lib, cfg: Config, quic: Option<QuicCfg>, log: &Arc<Log>) -> Result<Gateway, String> {
    ensure_forwarding(log);
    let filter = run::forward_filter(args.quic.is_some(), &args.own, &args.game_tcp, &args.game_udp);
    let handle = Arc::new(lib.open(&filter, windivert::LAYER_NETWORK_FORWARD, args.priority, 0)?);
    let redirects = match lib.open(watch::REDIRECT_FILTER, windivert::LAYER_NETWORK, args.priority, 0) {
        Ok(h) => Some(Arc::new(h)),
        Err(e) => {
            log.line(&format!("шлюз: сообщения «иди напрямую к роутеру» не перехватываются: {e}"));
            None
        }
    };
    let mut eng = Engine::new(cfg, run::forward_lists(args), Box::new(sys::random)).with_clock(sys::clock).with_mode(mode_for(&args.experiment));
    if let Some(q) = quic_for(quic) {
        eng = eng.with_quic(q);
    }
    eng.forward = true;
    eng.verbose = args.verbose;
    eng.track = args.live.is_some();
    let engine = Arc::new(Mutex::new(eng));
    let dropped = Arc::new(AtomicU64::new(0));
    let failed = Arc::new(AtomicU64::new(0));
    let mut threads = Vec::new();
    {
        let (h, engine, failed) = (handle.clone(), engine.clone(), failed.clone());
        threads.push(std::thread::spawn(move || {
            let mut buf = vec![0u8; windivert::MAX_PACKET];
            while let Some((len, addr)) = h.recv(&mut buf) {
                let raw = &buf[..len];
                let act = engine.lock().map(|mut e| e.handle(raw, addr, Instant::now())).unwrap_or(Action::Pass);
                match act {
                    Action::Pass => {
                        if !h.send(raw, &addr) {
                            failed.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    Action::Hold | Action::Drop => {}
                    Action::Send(out) => send_all(&h, out, &failed),
                    Action::Reject(..) => {}
                }
            }
        }));
    }
    if let Some(r) = redirects.clone() {
        let dropped = dropped.clone();
        threads.push(std::thread::spawn(move || {
            let mut buf = vec![0u8; 1600];
            while r.recv(&mut buf).is_some() {
                dropped.fetch_add(1, Ordering::Relaxed);
            }
        }));
    }
    log.line(&format!("шлюз включён: обрабатываю соединения устройств, для которых этот компьютер — шлюз, приём {}", mode_for(&args.experiment).name()));
    Ok(Gateway { handle, redirects, engine, threads: Mutex::new(threads), dropped, failed })
}

impl Gateway {
    pub fn shutters(&self) -> Vec<Shutter> {
        let mut v = vec![self.handle.shutter()];
        if let Some(r) = &self.redirects {
            v.push(r.shutter());
        }
        v
    }

    pub fn tick(&self, log: &Log) {
        let (out, notes) = self.engine.lock().map(|mut e| (e.expire(Instant::now()), std::mem::take(&mut e.notes))).unwrap_or_default();
        send_all(&self.handle, out, &self.failed);
        for n in notes {
            log.line(&format!("шлюз: {n}"));
        }
    }

    pub fn totals(&self) -> (u64, u64, u64) {
        self.engine.lock().map(|e| (e.stats.handled, e.stats.answered, e.stats.reset + e.stats.silent)).unwrap_or_default()
    }

    pub fn take_redeemed(&self) -> Vec<Instant> {
        self.engine.lock().map(|mut e| e.take_redeemed()).unwrap_or_default()
    }

    pub fn recent(&self) -> VecDeque<Decision> {
        self.engine.lock().map(|e| e.recent.clone()).unwrap_or_default()
    }

    pub fn stats_line(&self) -> String {
        let s = self.engine.lock().map(|e| run::stats_line(&e.stats, e.flows())).unwrap_or_default();
        format!(
            "шлюз: {s}; сообщений «иди к роутеру» выброшено {}, пакетов не отправлено {}",
            self.dropped.load(Ordering::Relaxed),
            self.failed.load(Ordering::Relaxed)
        )
    }

    pub fn finish(&self, log: &Log) {
        for t in self.threads.lock().map(|mut v| std::mem::take(&mut *v)).unwrap_or_default() {
            let _ = t.join();
        }
        let rest = self.engine.lock().map(|mut e| e.drain()).unwrap_or_default();
        send_all(&self.handle, rest, &self.failed);
        log.line(&self.stats_line());
    }
}

pub fn merge(main: &VecDeque<Decision>, gateway: Option<VecDeque<Decision>>) -> VecDeque<Decision> {
    let Some(g) = gateway.filter(|g| !g.is_empty()) else { return main.clone() };
    let mut all: Vec<Decision> = main.iter().cloned().chain(g).collect();
    all.sort_by(|a, b| a.time.cmp(&b.time));
    let skip = all.len().saturating_sub(RECENT);
    all.into_iter().skip(skip).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gateway_follows_chosen_mode() {
        assert!(matches!(mode_for("adaptive"), Mode::Adaptive(_)));
        assert_eq!(mode_for("sni-split"), Mode::Fixed(desync::SNI_SPLIT));
        assert_eq!(mode_for("nope"), Mode::Fixed(desync::GENERAL));
    }

    #[test]
    fn merge_keeps_time_order() {
        let d = |t: &str, id: u64| Decision { id, technique: None, outcome: None, time: t.into(), host: None, app: None, pid: None, handled: true, why: "", parts: 0 };
        let main: VecDeque<Decision> = vec![d("10:00:01", 1), d("10:00:05", 2)].into();
        let merged = merge(&main, Some(vec![d("10:00:03", 7)].into()));
        assert_eq!(merged.iter().map(|x| x.id).collect::<Vec<_>>(), vec![1, 7, 2]);
        assert_eq!(merge(&main, None), main);
    }
}
