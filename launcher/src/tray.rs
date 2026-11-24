use crate::core::paths::Paths;
use crate::core::service::{self, ServiceState, Status};
use crate::core::{settings, strategy};
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use windows_sys::Win32::UI::WindowsAndMessaging::{IsIconic, IsWindowVisible, SW_HIDE, SW_RESTORE, SW_SHOW, SetForegroundWindow, SetTimer, ShowWindow};

static HWND: AtomicIsize = AtomicIsize::new(0);
static STATE: Mutex<Option<View>> = Mutex::new(None);
static BUSY: AtomicBool = AtomicBool::new(false);
static EVENTS: Mutex<Vec<(bool, String)>> = Mutex::new(Vec::new());
static LAST_ACTION: Mutex<Option<Instant>> = Mutex::new(None);
static CTX: OnceLock<eframe::egui::Context> = OnceLock::new();
static QUIT: AtomicBool = AtomicBool::new(false);
static PHONE: AtomicBool = AtomicBool::new(false);
static NOTICE: AtomicU64 = AtomicU64::new(0);
const NOTICE_ID: u32 = 0x485A;

pub fn take_phone() -> bool {
    PHONE.swap(false, Ordering::Relaxed)
}

pub fn take_quit() -> bool {
    QUIT.swap(false, Ordering::Relaxed)
}

pub fn set_ctx(ctx: &eframe::egui::Context) {
    let _ = CTX.set(ctx.clone());
}

pub fn busy() -> bool {
    BUSY.load(Ordering::Relaxed)
}

pub fn claim() -> bool {
    !BUSY.swap(true, Ordering::Relaxed)
}

pub fn release() {
    BUSY.store(false, Ordering::Relaxed);
}

pub fn take_events() -> Vec<(bool, String)> {
    EVENTS.lock().map(|mut e| std::mem::take(&mut *e)).unwrap_or_default()
}

pub fn seconds_since_action() -> f64 {
    LAST_ACTION.lock().ok().and_then(|g| *g).map(|t| t.elapsed().as_secs_f64()).unwrap_or(f64::MAX)
}

fn wake() {
    if let Some(c) = CTX.get() {
        c.request_repaint();
    }
}

fn report(ok: bool, text: impl Into<String>) {
    if let Ok(mut e) = EVENTS.lock() {
        e.push((ok, text.into()));
    }
    wake();
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct View {
    pub on: bool,
    pub label: String,
}

impl View {
    pub fn from_status(s: &Status) -> View {
        let strategy = s.active_strategy().unwrap_or_default().to_string();
        let label = match s.service {
            ServiceState::Running if s.foreign_dir().is_some() => "Работает служба zapret из другой папки".to_string(),
            ServiceState::Running if strategy.is_empty() => "Защита активна".to_string(),
            ServiceState::Running => format!("Защита активна · {strategy}"),
            ServiceState::Stopped => "Служба остановлена".to_string(),
            ServiceState::NotInstalled => "Защита выключена".to_string(),
        };
        View { on: s.service == ServiceState::Running, label }
    }
}

struct TrayUi {
    icon: TrayIcon,
    status: MenuItem,
    toggle: MenuItem,
    tg: MenuItem,
    tg_shown: Option<bool>,
    lit: Icon,
    dim: Icon,
    shown: Option<View>,
}

thread_local! {
    static UI: RefCell<Option<TrayUi>> = const { RefCell::new(None) };
}

pub fn set_hwnd(h: isize) {
    HWND.store(h, Ordering::Relaxed);
}

pub fn window_visible() -> bool {
    let h = HWND.load(Ordering::Relaxed);
    h == 0 || unsafe { IsWindowVisible(h as _) != 0 && IsIconic(h as _) == 0 }
}

pub fn notify(title: &str, text: &str) {
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::Shell::{NIF_ICON, NIF_INFO, NIF_TIP, NIIF_WARNING, NIM_ADD, NIM_MODIFY, NOTIFYICONDATAW, Shell_NotifyIconW};
    use windows_sys::Win32::UI::WindowsAndMessaging::LoadIconW;
    let h = HWND.load(Ordering::Relaxed);
    if h == 0 {
        return;
    }
    fn fill(dst: &mut [u16], s: &str) {
        let max = dst.len() - 1;
        for (d, c) in dst.iter_mut().zip(s.encode_utf16().take(max)) {
            *d = c;
        }
    }
    unsafe {
        let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = h as _;
        nid.uID = NOTICE_ID;
        nid.uFlags = NIF_ICON | NIF_TIP | NIF_INFO;
        nid.hIcon = LoadIconW(GetModuleHandleW(std::ptr::null()), 1usize as _);
        nid.dwInfoFlags = NIIF_WARNING;
        fill(&mut nid.szTip, "HolyZapret");
        fill(&mut nid.szInfoTitle, title);
        fill(&mut nid.szInfo, text);
        if Shell_NotifyIconW(NIM_ADD, &nid) == 0 && Shell_NotifyIconW(NIM_MODIFY, &nid) == 0 {
            return;
        }
        let seq = NOTICE.fetch_add(1, Ordering::Relaxed) + 1;
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(12));
            if NOTICE.load(Ordering::Relaxed) == seq {
                clear_notice();
            }
        });
    }
}

pub fn clear_notice() {
    use windows_sys::Win32::UI::Shell::{NIM_DELETE, NOTIFYICONDATAW, Shell_NotifyIconW};
    let h = HWND.load(Ordering::Relaxed);
    if h == 0 {
        return;
    }
    unsafe {
        let mut del: NOTIFYICONDATAW = std::mem::zeroed();
        del.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        del.hWnd = h as _;
        del.uID = NOTICE_ID;
        Shell_NotifyIconW(NIM_DELETE, &del);
    }
}

pub fn exit() -> ! {
    clear_notice();
    std::process::exit(0)
}

pub fn hide_window() -> bool {
    let h = HWND.load(Ordering::Relaxed);
    h != 0 && unsafe {
        ShowWindow(h as _, SW_HIDE);
        true
    }
}

pub fn show_window() {
    let h = HWND.load(Ordering::Relaxed);
    if h != 0 {
        unsafe {
            ShowWindow(h as _, SW_SHOW);
            ShowWindow(h as _, SW_RESTORE);
            SetForegroundWindow(h as _);
        }
    }
}

pub fn publish(s: &Status) {
    if let Ok(mut g) = STATE.lock() {
        *g = Some(View::from_status(s));
    }
}

fn dimmed(rgba: &[u8]) -> Vec<u8> {
    rgba.chunks(4)
        .flat_map(|p| {
            let l = (0.3 * p[0] as f32 + 0.59 * p[1] as f32 + 0.11 * p[2] as f32) as u8;
            [l, l, l, (p[3] as f32 * 0.55) as u8]
        })
        .collect()
}

fn sync() {
    let view = STATE.lock().ok().and_then(|g| g.clone());
    UI.with(|u| {
        let mut u = u.borrow_mut();
        let Some(ui) = u.as_mut() else { return };
        let tg = crate::core::tgproxy::control::running();
        if ui.tg_shown != Some(tg) {
            ui.tg.set_text(if tg { "Выключить Telegram-прокси" } else { "Включить Telegram-прокси" });
            ui.tg_shown = Some(tg);
        }
        if view.is_none() || ui.shown == view {
            return;
        }
        let v = view.unwrap();
        let _ = ui.icon.set_icon(Some(if v.on { ui.lit.clone() } else { ui.dim.clone() }));
        let _ = ui.icon.set_tooltip(Some(format!("HolyZapret · {}", v.label)));
        ui.status.set_text(&v.label);
        ui.toggle.set_text(if v.on { "Выключить защиту" } else { "Включить защиту" });
        ui.shown = Some(v);
    });
}

unsafe extern "system" fn on_timer(_: windows_sys::Win32::Foundation::HWND, _: u32, _: usize, _: u32) {
    sync();
}

fn toggle(paths: Paths) {
    if BUSY.swap(true, Ordering::Relaxed) {
        return;
    }
    if let Ok(mut g) = LAST_ACTION.lock() {
        *g = Some(Instant::now());
    }
    wake();
    std::thread::spawn(move || {
        let st = service::status(&paths);
        match st.service {
            ServiceState::Running => report(true, format!("Из трея: {}", service::remove())),
            ServiceState::Stopped => match service::start() {
                Ok(()) => report(true, "Из трея: служба запущена"),
                Err(e) => report(false, format!("Из трея: не удалось запустить службу: {e}")),
            },
            ServiceState::NotInstalled => {
                let last = settings::last_strategy();
                let found = last.and_then(|id| strategy::discover(&paths).into_iter().find(|s| s.matches(&id)));
                match found {
                    Some(s) => match service::install(&paths, &s) {
                        Ok(m) => report(true, format!("Из трея: {m}")),
                        Err(e) => report(false, format!("Из трея: ошибка установки: {e}")),
                    },
                    None => {
                        report(false, "Выберите стратегию: из трея включается только последняя установленная");
                        show_window();
                    }
                }
            }
        }
        publish(&service::status(&paths));
        if let Ok(mut g) = LAST_ACTION.lock() {
            *g = Some(Instant::now());
        }
        BUSY.store(false, Ordering::Relaxed);
        wake();
    });
}

pub fn create(rgba: Vec<u8>, w: u32, h: u32, paths: Option<Paths>) -> bool {
    let (Ok(lit), Ok(dim)) = (Icon::from_rgba(rgba.clone(), w, h), Icon::from_rgba(dimmed(&rgba), w, h)) else {
        return false;
    };
    let menu = Menu::new();
    let status = MenuItem::new("HolyZapret", false, None);
    let toggle_item = MenuItem::new("Включить защиту", paths.is_some(), None);
    let tg_item = MenuItem::new("Включить Telegram-прокси", true, None);
    let phone = MenuItem::new("Раздача на телефон…", true, None);
    let open = MenuItem::new("Открыть HolyZapret", true, None);
    let quit = MenuItem::new("Выход", true, None);
    let sep = PredefinedMenuItem::separator();
    let sep2 = PredefinedMenuItem::separator();
    if menu.append_items(&[&status, &sep, &toggle_item, &tg_item, &phone, &open, &sep2, &quit]).is_err() {
        return false;
    }
    let (open_id, quit_id, toggle_id) = (open.id().clone(), quit.id().clone(), toggle_item.id().clone());
    let tg_id = tg_item.id().clone();
    let phone_id = phone.id().clone();
    MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
        if e.id == open_id {
            show_window();
        } else if e.id == phone_id {
            PHONE.store(true, Ordering::Relaxed);
            show_window();
            wake();
        } else if e.id == quit_id {
            if crate::core::lanproxy::control::running() {
                QUIT.store(true, Ordering::Relaxed);
                show_window();
                wake();
            } else {
                exit();
            }
        } else if e.id == tg_id {
            std::thread::spawn(|| {
                let on = !crate::core::tgproxy::control::running();
                match crate::core::tgproxy::control::enable(on) {
                    Ok(m) => report(true, format!("Из трея: {m}")),
                    Err(e) => report(false, format!("Из трея: Telegram-прокси не запустился: {e}")),
                }
            });
        } else if e.id == toggle_id {
            if let Some(p) = paths.clone() {
                toggle(p);
            }
        }
    }));
    TrayIconEvent::set_event_handler(Some(|e: TrayIconEvent| {
        if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
            show_window();
        }
    }));
    let Ok(icon) = TrayIconBuilder::new().with_menu(Box::new(menu)).with_tooltip("HolyZapret").with_icon(dim.clone()).build() else {
        return false;
    };
    UI.with(|u| *u.borrow_mut() = Some(TrayUi { icon, status, toggle: toggle_item, tg: tg_item, tg_shown: None, lit, dim, shown: None }));
    unsafe {
        SetTimer(std::ptr::null_mut(), 0, 1500, Some(on_timer));
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(service: ServiceState, strategy: Option<&str>) -> Status {
        Status {
            service,
            windivert: ServiceState::NotInstalled,
            engine_running: false,
            installed_strategy: strategy.map(String::from),
            game: Default::default(),
            owner: crate::core::service::Owner::Ours,
            foreign_engines: Vec::new(),
        }
    }

    #[test]
    fn views() {
        let v = View::from_status(&status(ServiceState::Running, Some("flowseal\\general")));
        assert!(v.on);
        assert_eq!(v.label, "Защита активна · flowseal\\general");
        let v = View::from_status(&status(ServiceState::NotInstalled, None));
        assert!(!v.on);
        assert_eq!(v.label, "Защита выключена");
        assert_eq!(dimmed(&[255, 0, 0, 200]), vec![76, 76, 76, 110]);
    }
}
