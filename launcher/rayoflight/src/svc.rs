use std::sync::Mutex;
use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering};
use windows_sys::Win32::System::Services::{
    CloseServiceHandle, OpenSCManagerW, OpenServiceW, QueryServiceStatus, RegisterServiceCtrlHandlerExW, SC_MANAGER_CONNECT, SERVICE_ACCEPT_SHUTDOWN,
    SERVICE_ACCEPT_STOP, SERVICE_CONTROL_INTERROGATE, SERVICE_CONTROL_SHUTDOWN, SERVICE_CONTROL_STOP, SERVICE_QUERY_STATUS, SERVICE_RUNNING,
    SERVICE_START_PENDING, SERVICE_STATUS, SERVICE_STOP_PENDING, SERVICE_STOPPED, SERVICE_TABLE_ENTRYW, SERVICE_WIN32_OWN_PROCESS, SetServiceStatus,
    StartServiceCtrlDispatcherW,
};

pub const NAME: &str = "rayoflight";

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

pub fn running(name: &str) -> bool {
    let n = wide(name);
    unsafe {
        let scm = OpenSCManagerW(std::ptr::null(), std::ptr::null(), SC_MANAGER_CONNECT);
        if scm.is_null() {
            return false;
        }
        let svc = OpenServiceW(scm, n.as_ptr(), SERVICE_QUERY_STATUS);
        let mut st: SERVICE_STATUS = std::mem::zeroed();
        let ok = !svc.is_null() && QueryServiceStatus(svc, &mut st) != 0;
        if !svc.is_null() {
            CloseServiceHandle(svc);
        }
        CloseServiceHandle(scm);
        ok && st.dwCurrentState == SERVICE_RUNNING
    }
}

static HANDLE: AtomicIsize = AtomicIsize::new(0);
static CHECKPOINT: AtomicU32 = AtomicU32::new(0);
static BODY: Mutex<Option<Box<dyn FnOnce() -> i32 + Send>>> = Mutex::new(None);

fn report(state: u32, exit: u32) {
    let h = HANDLE.load(Ordering::SeqCst);
    if h == 0 {
        return;
    }
    let accept = if state == SERVICE_RUNNING { SERVICE_ACCEPT_STOP | SERVICE_ACCEPT_SHUTDOWN } else { 0 };
    let pending = state == SERVICE_START_PENDING || state == SERVICE_STOP_PENDING;
    let st = SERVICE_STATUS {
        dwServiceType: SERVICE_WIN32_OWN_PROCESS,
        dwCurrentState: state,
        dwControlsAccepted: accept,
        dwWin32ExitCode: exit,
        dwServiceSpecificExitCode: 0,
        dwCheckPoint: if pending { CHECKPOINT.fetch_add(1, Ordering::SeqCst) + 1 } else { 0 },
        dwWaitHint: if pending { 5000 } else { 0 },
    };
    unsafe { SetServiceStatus(h as _, &st) };
}

unsafe extern "system" fn handler(control: u32, _: u32, _: *mut core::ffi::c_void, _: *mut core::ffi::c_void) -> u32 {
    match control {
        SERVICE_CONTROL_STOP | SERVICE_CONTROL_SHUTDOWN => {
            report(SERVICE_STOP_PENDING, 0);
            crate::sys::request_stop();
        }
        SERVICE_CONTROL_INTERROGATE => {}
        _ => {}
    }
    0
}

unsafe extern "system" fn service_main(_: u32, _: *mut windows_sys::core::PWSTR) {
    let n = wide(NAME);
    let h = unsafe { RegisterServiceCtrlHandlerExW(n.as_ptr(), Some(handler), std::ptr::null()) };
    if h.is_null() {
        return;
    }
    HANDLE.store(h as isize, Ordering::SeqCst);
    report(SERVICE_START_PENDING, 0);
    report(SERVICE_RUNNING, 0);
    let body = BODY.lock().ok().and_then(|mut b| b.take());
    let code = body.map(|f| f()).unwrap_or(1);
    report(SERVICE_STOPPED, if code == 0 { 0 } else { 1066 });
}

pub fn dispatch(body: impl FnOnce() -> i32 + Send + 'static) -> Result<(), String> {
    if let Ok(mut b) = BODY.lock() {
        *b = Some(Box::new(body));
    }
    let mut n = wide(NAME);
    let table = [SERVICE_TABLE_ENTRYW { lpServiceName: n.as_mut_ptr(), lpServiceProc: Some(service_main) }, SERVICE_TABLE_ENTRYW { lpServiceName: std::ptr::null_mut(), lpServiceProc: None }];
    if unsafe { StartServiceCtrlDispatcherW(table.as_ptr()) } == 0 {
        return Err("запуск как службы возможен только из диспетчера служб Windows".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queries_services() {
        assert!(running("EventLog") || running("Dnscache") || running("RpcSs"));
        assert!(!running("нет-такой-службы-hz"));
    }

    #[test]
    fn dispatch_outside_scm_fails() {
        assert!(dispatch(|| 0).is_err());
    }
}
