use windows_sys::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError};
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::WindowsAndMessaging::{IsWindow, SW_RESTORE, SW_SHOW, SetForegroundWindow, ShowWindow};
use winreg::RegKey;
use winreg::enums::HKEY_CURRENT_USER;

const KEY: &str = r"Software\HolyZapret";

pub fn acquire() -> bool {
    let name: Vec<u16> = "Local\\HolyZapretLauncher\0".encode_utf16().collect();
    unsafe {
        let h = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
        if h.is_null() {
            return false;
        }
        if GetLastError() == ERROR_ALREADY_EXISTS {
            CloseHandle(h);
            return false;
        }
        true
    }
}

pub fn remember(hwnd: isize) {
    if let Ok((k, _)) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(KEY) {
        let _ = k.set_value("Window", &(hwnd as u64));
    }
}

pub fn activate_existing() {
    let Ok(k) = RegKey::predef(HKEY_CURRENT_USER).open_subkey(KEY) else { return };
    let Ok(h) = k.get_value::<u64, _>("Window") else { return };
    let h = h as isize as _;
    unsafe {
        if IsWindow(h) != 0 {
            ShowWindow(h, SW_SHOW);
            ShowWindow(h, SW_RESTORE);
            SetForegroundWindow(h);
        }
    }
}
