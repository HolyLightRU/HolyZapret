use super::paths::Paths;
use super::{ray, settings, tgwarp, warp};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const TICK: Duration = Duration::from_secs(5);
const IDLE: Duration = Duration::from_secs(5 * 60);
const FRESH: Duration = Duration::from_secs(10);

static OWNED: Mutex<Option<warp::Mode>> = Mutex::new(None);
static PATHS: std::sync::OnceLock<Paths> = std::sync::OnceLock::new();

pub fn mode_name(m: warp::Mode) -> &'static str {
    match m {
        warp::Mode::Warp => "warp",
        warp::Mode::Doh => "doh",
        warp::Mode::Proxy => "proxy",
        warp::Mode::Other => "other",
    }
}

pub fn mode_from(name: &str) -> Option<warp::Mode> {
    match name {
        "warp" => Some(warp::Mode::Warp),
        "doh" => Some(warp::Mode::Doh),
        "proxy" => Some(warp::Mode::Proxy),
        "other" => Some(warp::Mode::Other),
        _ => None,
    }
}

fn remember(mode: Option<warp::Mode>) {
    settings::set_warp_owned(mode.map(mode_name));
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Nothing,
    Connect { switch: bool },
}

pub fn plan(state: &warp::State) -> Step {
    if !state.installed || state.needs_setup {
        return Step::Nothing;
    }
    match state.conn {
        warp::Conn::Connected | warp::Conn::Connecting => Step::Nothing,
        warp::Conn::Disconnected | warp::Conn::Unknown => Step::Connect { switch: state.mode != warp::Mode::Proxy },
    }
}

pub fn owned() -> bool {
    OWNED.lock().is_ok_and(|o| o.is_some())
}

fn acquire(log: &dyn Fn(String)) {
    let state = warp::query();
    let Step::Connect { switch } = plan(&state) else { return };
    if switch {
        if let Err(e) = warp::set_mode(warp::Mode::Proxy) {
            log(format!("WARP для сайтов: не удалось включить режим прокси: {e}"));
            return;
        }
    }
    match warp::connect() {
        Ok(()) => {
            if let Ok(mut o) = OWNED.lock() {
                remember(Some(*o.get_or_insert(state.mode)));
            }
            log("WARP для сайтов: WARP подключён в режиме прокси, он нужен сайтам, которые не открылись другими способами".into());
        }
        Err(e) => log(format!("WARP для сайтов: WARP не подключился: {e}")),
    }
}

pub fn release(log: &dyn Fn(String)) {
    let Some(mode) = OWNED.lock().ok().and_then(|mut o| o.take()) else { return };
    remember(None);
    if tgwarp::enabled() {
        return;
    }
    let _ = warp::disconnect();
    if mode != warp::Mode::Proxy && mode != warp::Mode::Other {
        let _ = warp::set_mode(mode);
    }
    log("WARP для сайтов: WARP отключён, сайтам через WARP он больше не нужен".into());
}

fn needed(paths: &Paths) -> bool {
    ray::read_live(paths).filter(|(l, age)| l.running && *age < FRESH).and_then(|(l, _)| l.warp).is_some_and(|w| w.wanted || w.active > 0)
}

pub fn on_exit() {
    if !owned() || PATHS.get().is_some_and(needed) {
        return;
    }
    release(&|_| {});
}

pub fn spawn(paths: Paths, log: impl Fn(String) + Send + 'static) {
    let _ = PATHS.set(paths.clone());
    if let Some(m) = settings::warp_owned().and_then(|n| mode_from(&n)) {
        if let Ok(mut o) = OWNED.lock() {
            o.get_or_insert(m);
        }
    }
    std::thread::spawn(move || {
        let mut idle_since: Option<Instant> = None;
        loop {
            std::thread::sleep(TICK);
            if !settings::ray_warp() && !needed(&paths) {
                release(&log);
                idle_since = None;
                continue;
            }
            let live = ray::read_live(&paths).filter(|(l, age)| l.running && *age < FRESH).and_then(|(l, _)| l.warp);
            let busy = live.as_ref().is_some_and(|w| w.wanted || w.active > 0);
            if busy {
                idle_since = None;
                if !live.as_ref().is_some_and(|w| w.ready) {
                    acquire(&log);
                }
                continue;
            }
            if owned() && idle_since.get_or_insert_with(Instant::now).elapsed() >= IDLE {
                release(&log);
                idle_since = None;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(conn: warp::Conn, mode: warp::Mode) -> warp::State {
        warp::State { installed: true, conn, mode, reason: String::new(), needs_setup: false }
    }

    #[test]
    fn connects_only_when_warp_is_idle() {
        assert_eq!(plan(&warp::State::missing()), Step::Nothing);
        assert_eq!(plan(&st(warp::Conn::Connected, warp::Mode::Warp)), Step::Nothing);
        assert_eq!(plan(&st(warp::Conn::Connected, warp::Mode::Proxy)), Step::Nothing);
        assert_eq!(plan(&st(warp::Conn::Disconnected, warp::Mode::Warp)), Step::Connect { switch: true });
        assert_eq!(plan(&st(warp::Conn::Disconnected, warp::Mode::Proxy)), Step::Connect { switch: false });
        let mut setup = st(warp::Conn::Disconnected, warp::Mode::Warp);
        setup.needs_setup = true;
        assert_eq!(plan(&setup), Step::Nothing);
    }

    #[test]
    fn remembered_mode_survives_a_restart_by_name() {
        for m in [warp::Mode::Warp, warp::Mode::Doh, warp::Mode::Proxy, warp::Mode::Other] {
            assert_eq!(mode_from(mode_name(m)), Some(m));
        }
        assert_eq!(mode_from(""), None);
    }
}
