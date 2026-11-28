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
static WORKING: AtomicBool = AtomicBool::new(false);
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

pub fn set_working(v: bool) {
    WORKING.store(v, Ordering::Relaxed);
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

fn mode_label(experiment: &str) -> String {
    if experiment == "adaptive" {
        "самоподстройка".to_string()
    } else {
        crate::core::ray::experiment_label(experiment).to_string()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct View {
    pub on: bool,
    pub label: String,
}

impl View {
    pub fn from_status(s: &Status) -> View {
        let label = match s.service {
            ServiceState::Running if s.foreign_dir().is_some() => "Работает служба zapret из другой папки".to_string(),
            ServiceState::Running if !s.engine_running => "Служба запущена, но движок не работает".to_string(),
            ServiceState::Running => format!("Защита активна · {}", mode_label(&settings::ray_experiment())),
            ServiceState::Stopped => "Служба остановлена".to_string(),
            ServiceState::NotInstalled => "Защита выключена".to_string(),
        };
        View { on: s.service == ServiceState::Running && s.engine_running, label }
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
    balloon(title, text, windows_sys::Win32::UI::Shell::NIIF_WARNING);
}

pub fn inform(title: &str, text: &str) {
    balloon(title, text, windows_sys::Win32::UI::Shell::NIIF_INFO);
}

fn balloon(title: &str, text: &str, kind: u32) {
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::Shell::{NIF_ICON, NIF_INFO, NIF_TIP, NIM_ADD, NIM_MODIFY, NOTIFYICONDATAW, Shell_NotifyIconW};
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
        nid.dwInfoFlags = kind;
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
    crate::core::warpauto::on_exit();
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
    if h == 0 {
        return;
    }
    if COMPACT.load(Ordering::Relaxed) {
        FULL.store(true, Ordering::Relaxed);
        cloak(true);
        unsafe { ShowWindow(h as _, SW_HIDE) };
        tool_window(false);
        topmost(false);
    }
    unsafe {
        ShowWindow(h as _, SW_SHOW);
        ShowWindow(h as _, SW_RESTORE);
        SetForegroundWindow(h as _);
    }
    wake();
}

static COMPACT: AtomicBool = AtomicBool::new(false);
static FULL: AtomicBool = AtomicBool::new(false);
static FLY: Mutex<Option<(i32, i32, bool)>> = Mutex::new(None);
static FLY_CLOSED: Mutex<Option<Instant>> = Mutex::new(None);

pub fn set_compact(v: bool) {
    COMPACT.store(v, Ordering::Relaxed);
}

pub fn take_full() -> bool {
    FULL.swap(false, Ordering::Relaxed)
}

pub fn take_fly() -> Option<(i32, i32, bool)> {
    FLY.lock().ok().and_then(|mut g| g.take())
}

pub fn fly_closed() {
    if let Ok(mut g) = FLY_CLOSED.lock() {
        *g = Some(Instant::now());
    }
}

fn open_fly(x: i32, y: i32, menu: bool) {
    let h = HWND.load(Ordering::Relaxed);
    if h == 0 {
        return;
    }
    if FLY_CLOSED.lock().ok().and_then(|g| *g).is_some_and(|t| t.elapsed().as_millis() < 350) {
        return;
    }
    if COMPACT.load(Ordering::Relaxed) && window_visible() {
        hide_window();
        fly_closed();
        return;
    }
    if window_visible() {
        show_window();
        return;
    }
    if let Ok(mut g) = FLY.lock() {
        *g = Some((x, y, menu));
    }
    cloak(true);
    tool_window(true);
    unsafe {
        ShowWindow(h as _, SW_SHOW);
        ShowWindow(h as _, SW_RESTORE);
        SetForegroundWindow(h as _);
    }
    topmost(true);
    wake();
}

pub fn cloak(on: bool) {
    use windows_sys::Win32::Graphics::Dwm::{DWMWA_CLOAK, DwmSetWindowAttribute};
    let h = HWND.load(Ordering::Relaxed);
    if h == 0 {
        return;
    }
    let v: i32 = on as i32;
    unsafe {
        DwmSetWindowAttribute(h as _, DWMWA_CLOAK as _, &v as *const i32 as _, 4);
    }
}

fn tool_window(on: bool) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GWL_EXSTYLE, GetWindowLongPtrW, SetWindowLongPtrW, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW};
    let h = HWND.load(Ordering::Relaxed);
    if h == 0 {
        return;
    }
    use windows_sys::Win32::UI::WindowsAndMessaging::{GWL_STYLE, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos, WS_SYSMENU};
    unsafe {
        let style = GetWindowLongPtrW(h as _, GWL_EXSTYLE);
        let next = if on { (style | WS_EX_TOOLWINDOW as isize) & !(WS_EX_APPWINDOW as isize) } else { style & !(WS_EX_TOOLWINDOW as isize) };
        if next != style {
            SetWindowLongPtrW(h as _, GWL_EXSTYLE, next);
        }
        let base = GetWindowLongPtrW(h as _, GWL_STYLE);
        if on {
            if base & WS_SYSMENU as isize != 0 {
                SYSMENU.store(true, Ordering::Relaxed);
                SetWindowLongPtrW(h as _, GWL_STYLE, base & !(WS_SYSMENU as isize));
            }
        } else if SYSMENU.swap(false, Ordering::Relaxed) {
            SetWindowLongPtrW(h as _, GWL_STYLE, base | WS_SYSMENU as isize);
        }
        SetWindowPos(h as _, std::ptr::null_mut(), 0, 0, 0, 0, SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
    }
}

static SYSMENU: AtomicBool = AtomicBool::new(false);
static POP: Mutex<Option<(i32, i32)>> = Mutex::new(None);
static POP_CLOSED: Mutex<Option<Instant>> = Mutex::new(None);

pub fn take_pop() -> Option<(i32, i32)> {
    POP.lock().ok().and_then(|mut g| g.take())
}

pub fn pop_closed() {
    if let Ok(mut g) = POP_CLOSED.lock() {
        *g = Some(Instant::now());
    }
}

pub fn popup_ready(title: &str) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, GWL_STYLE, GetWindowLongPtrW, SetWindowLongPtrW, WS_SYSMENU};
    let name: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
    unsafe {
        let h = FindWindowW(std::ptr::null(), name.as_ptr());
        if h.is_null() {
            return false;
        }
        let style = GetWindowLongPtrW(h, GWL_STYLE);
        SetWindowLongPtrW(h, GWL_STYLE, style & !(WS_SYSMENU as isize));
        crate::window::popup_backdrop(h as isize);
        SetForegroundWindow(h);
        true
    }
}

pub fn topmost(on: bool) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{HWND_NOTOPMOST, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowPos};
    let h = HWND.load(Ordering::Relaxed);
    if h == 0 {
        return;
    }
    unsafe {
        SetWindowPos(h as _, if on { HWND_TOPMOST } else { HWND_NOTOPMOST }, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    }
}

pub fn work_area(x: i32, y: i32) -> Option<([i32; 4], [i32; 4])> {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint};
    unsafe {
        let m = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(m, &mut info) == 0 {
            return None;
        }
        let r = |r: windows_sys::Win32::Foundation::RECT| [r.left, r.top, r.right, r.bottom];
        Some((r(info.rcMonitor), r(info.rcWork)))
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
        let Ok(mut u) = u.try_borrow_mut() else { return };
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
    if WORKING.load(Ordering::Relaxed) {
        report(false, "Из трея: сейчас идёт подбор, обновление или другое действие со службой, попробуйте, когда оно закончится");
        return;
    }
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
            ServiceState::Running => match service::remove() {
                Ok(m) => report(true, format!("Из трея: {m}")),
                Err(e) => report(false, format!("Из трея: {e}")),
            },
            ServiceState::Stopped => match service::start() {
                Ok(()) => report(true, "Из трея: служба запущена"),
                Err(e) => report(false, format!("Из трея: не удалось запустить службу: {e}")),
            },
            ServiceState::NotInstalled => {
                let last = settings::last_strategy();
                let all = strategy::discover(&paths);
                let found = last.and_then(|id| all.iter().find(|s| s.matches(&id)).cloned()).or_else(|| all.iter().find(|s| s.id == crate::core::ray::DEFAULT_ID).cloned());
                match found {
                    Some(s) => match service::install(&paths, &s) {
                        Ok(m) => report(true, format!("Из трея: {m}")),
                        Err(e) => report(false, format!("Из трея: ошибка установки: {e}")),
                    },
                    None => {
                        report(false, "Нет файла bin\\RayOfLight.exe: откройте HolyZapret и скачайте его заново");
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
    let phone = MenuItem::new("Устройства…", true, None);
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
            if crate::core::lanproxy::control::running() || WORKING.load(Ordering::Relaxed) || busy() {
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
        let TrayIconEvent::Click { button, button_state: MouseButtonState::Up, position, .. } = e else { return };
        let (x, y) = (position.x as i32, position.y as i32);
        match button {
            MouseButton::Left => open_fly(x, y, false),
            MouseButton::Right if window_visible() && !COMPACT.load(Ordering::Relaxed) => {
                if POP_CLOSED.lock().ok().and_then(|g| *g).is_some_and(|t| t.elapsed().as_millis() < 350) {
                    return;
                }
                if let Ok(mut g) = POP.lock() {
                    *g = Some((x, y));
                }
                wake();
            }
            MouseButton::Right => open_fly(x, y, true),
            _ => {}
        }
    }));
    let Ok(icon) = TrayIconBuilder::new().with_menu(Box::new(menu)).with_menu_on_left_click(false).with_menu_on_right_click(false).with_tooltip("HolyZapret").with_icon(dim.clone()).build() else {
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
        let v = View::from_status(&status(ServiceState::Running, Some("rayoflight:adaptive")));
        assert!(!v.on);
        assert_eq!(v.label, "Служба запущена, но движок не работает");
        let mut s = status(ServiceState::Running, Some("rayoflight:adaptive"));
        s.engine_running = true;
        let v = View::from_status(&s);
        assert!(v.on);
        assert!(v.label.starts_with("Защита активна · "));
        assert!(!v.label.contains("rayoflight:"));
        assert_eq!(mode_label("adaptive"), "самоподстройка");
        let v = View::from_status(&status(ServiceState::NotInstalled, None));
        assert!(!v.on);
        assert_eq!(v.label, "Защита выключена");
        assert_eq!(dimmed(&[255, 0, 0, 200]), vec![76, 76, 76, 110]);
    }
}
