use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;

const DWMWA_USE_IMMERSIVE_DARK_MODE: u32 = 20;
const DWMWA_WINDOW_CORNER_PREFERENCE: u32 = 33;
const CORNER_ROUND: i32 = 2;
const ACRYLIC_SMOOTH_FROM: u32 = 22621;

fn set_attr(hwnd: isize, attr: u32, value: i32) {
    unsafe {
        DwmSetWindowAttribute(hwnd as _, attr, &value as *const i32 as _, 4);
    }
}

pub fn apply_backdrop(frame: &impl raw_window_handle::HasWindowHandle, hwnd: isize) {
    set_attr(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, 1);
    set_attr(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, CORNER_ROUND);
    let tint = Some((6, 6, 10, 215));
    if crate::core::picker::windows_build() >= ACRYLIC_SMOOTH_FROM {
        let _ = window_vibrancy::apply_acrylic(frame, tint);
    } else {
        let _ = window_vibrancy::apply_blur(frame, tint);
    }
}
