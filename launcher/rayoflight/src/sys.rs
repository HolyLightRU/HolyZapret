use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, SYSTEMTIME};

pub fn random(buf: &mut [u8]) {
    use windows_sys::Win32::Security::Cryptography::{BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom};
    let ok = unsafe { BCryptGenRandom(std::ptr::null_mut(), buf.as_mut_ptr(), buf.len() as u32, BCRYPT_USE_SYSTEM_PREFERRED_RNG) } == 0;
    if !ok {
        let mut x = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0x9E37_79B9_7F4A_7C15) | 1;
        for b in buf.iter_mut() {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            *b = x as u8;
        }
    }
}

pub fn utf8_console() {
    use windows_sys::Win32::System::Console::SetConsoleOutputCP;
    unsafe { SetConsoleOutputCP(65001) };
}

pub fn cpu_time() -> u64 {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
    let z = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
    let (mut c, mut e, mut k, mut u) = (z, z, z, z);
    if unsafe { GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u) } == 0 {
        return 0;
    }
    let v = |f: FILETIME| ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64;
    v(k) + v(u)
}

pub fn memory() -> u64 {
    use windows_sys::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    let mut m: PROCESS_MEMORY_COUNTERS = unsafe { std::mem::zeroed() };
    m.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
    if unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut m, m.cb) } == 0 {
        return 0;
    }
    m.WorkingSetSize as u64
}

pub fn clock() -> String {
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;
    let mut t: SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe { GetLocalTime(&mut t) };
    format!("{:02}:{:02}:{:02}", t.wHour, t.wMinute, t.wSecond)
}

pub struct GlobalLock(HANDLE);

impl GlobalLock {
    pub fn take(name: &str) -> Option<GlobalLock> {
        use windows_sys::Win32::System::Threading::{CreateMutexA, INFINITE, WaitForSingleObject};
        let n: Vec<u8> = name.bytes().chain(Some(0)).collect();
        let h = unsafe { CreateMutexA(std::ptr::null(), 0, n.as_ptr()) };
        if h.is_null() {
            return None;
        }
        unsafe { WaitForSingleObject(h, INFINITE) };
        Some(GlobalLock(h))
    }
}

impl Drop for GlobalLock {
    fn drop(&mut self) {
        use windows_sys::Win32::System::Threading::ReleaseMutex;
        unsafe {
            ReleaseMutex(self.0);
            CloseHandle(self.0);
        }
    }
}

static STOP: AtomicBool = AtomicBool::new(false);
static ON_STOP: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

unsafe extern "system" fn on_ctrl(_: u32) -> windows_sys::core::BOOL {
    request_stop();
    1
}

pub fn on_stop(f: impl Fn() + Send + Sync + 'static) {
    use windows_sys::Win32::System::Console::SetConsoleCtrlHandler;
    let _ = ON_STOP.set(Box::new(f));
    unsafe { SetConsoleCtrlHandler(Some(on_ctrl), 1) };
}

pub fn request_stop() {
    if !STOP.swap(true, Ordering::SeqCst) {
        if let Some(f) = ON_STOP.get() {
            f();
        }
    }
}

pub fn stopping() -> bool {
    STOP.load(Ordering::SeqCst)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_and_clock() {
        let mut a = [0u8; 32];
        let mut b = [0u8; 32];
        random(&mut a);
        random(&mut b);
        assert_ne!(a, b);
        let c = clock();
        assert_eq!(c.len(), 8);
        assert_eq!(c.as_bytes()[2], b':');
        let l = GlobalLock::take("Local\\rayoflight-test-lock");
        assert!(l.is_some());
    }
}
