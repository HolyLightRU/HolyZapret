use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};

const DWMWA_USE_IMMERSIVE_DARK_MODE: u32 = 20;
const DWMWA_WINDOW_CORNER_PREFERENCE: u32 = 33;
const DWMWA_SYSTEMBACKDROP_TYPE: u32 = 38;
const DWMSBT_NONE: i32 = 1;
const CORNER_ROUND: i32 = 2;
const ACRYLIC_SMOOTH_FROM: u32 = 22621;
const WCA_ACCENT_POLICY: u32 = 19;
const ACCENT_ENABLE_ACRYLICBLURBEHIND: u32 = 4;
const GLASS_TINT: (u8, u8, u8, u8) = (12, 9, 18, 150);

#[repr(C)]
struct AccentPolicy {
    state: u32,
    flags: u32,
    color: u32,
    animation: u32,
}

#[repr(C)]
struct CompositionData {
    attribute: u32,
    data: *mut core::ffi::c_void,
    size: usize,
}

type SetComposition = unsafe extern "system" fn(isize, *mut CompositionData) -> i32;

fn set_attr(hwnd: isize, attr: u32, value: i32) {
    unsafe {
        DwmSetWindowAttribute(hwnd as _, attr, &value as *const i32 as _, 4);
    }
}

fn glass(hwnd: isize, (r, g, b, a): (u8, u8, u8, u8)) -> bool {
    let name: Vec<u16> = "user32.dll\0".encode_utf16().collect();
    unsafe {
        let module = GetModuleHandleW(name.as_ptr());
        if module.is_null() {
            return false;
        }
        let Some(proc) = GetProcAddress(module, b"SetWindowCompositionAttribute\0".as_ptr()) else {
            return false;
        };
        let set: SetComposition = std::mem::transmute(proc);
        let mut policy = AccentPolicy {
            state: ACCENT_ENABLE_ACRYLICBLURBEHIND,
            flags: 2,
            color: (a as u32) << 24 | (b as u32) << 16 | (g as u32) << 8 | r as u32,
            animation: 0,
        };
        let mut data = CompositionData {
            attribute: WCA_ACCENT_POLICY,
            data: &mut policy as *mut AccentPolicy as *mut _,
            size: std::mem::size_of::<AccentPolicy>(),
        };
        set(hwnd, &mut data) != 0
    }
}

pub fn popup_backdrop(hwnd: isize) {
    set_attr(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, 1);
    set_attr(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, CORNER_ROUND);
    if crate::core::picker::windows_build() >= ACRYLIC_SMOOTH_FROM {
        set_attr(hwnd, DWMWA_SYSTEMBACKDROP_TYPE, DWMSBT_NONE);
        glass(hwnd, GLASS_TINT);
    }
}

pub fn apply_backdrop(frame: &impl raw_window_handle::HasWindowHandle, hwnd: isize) {
    set_attr(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, 1);
    set_attr(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, CORNER_ROUND);
    let tint = Some((6, 6, 10, 215));
    if crate::core::picker::windows_build() >= ACRYLIC_SMOOTH_FROM {
        set_attr(hwnd, DWMWA_SYSTEMBACKDROP_TYPE, DWMSBT_NONE);
        if !glass(hwnd, GLASS_TINT) {
            let _ = window_vibrancy::apply_acrylic(frame, tint);
        }
    } else {
        let _ = window_vibrancy::apply_blur(frame, tint);
    }
}
