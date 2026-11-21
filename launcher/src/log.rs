use windows_sys::Win32::Foundation::SYSTEMTIME;
use windows_sys::Win32::System::SystemInformation::GetLocalTime;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lvl {
    Info,
    Ok,
    Err,
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub time: String,
    pub lvl: Lvl,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct Toast {
    pub lvl: Lvl,
    pub text: String,
    pub born: f64,
}

pub fn now_hms() -> String {
    let mut t: SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe { GetLocalTime(&mut t) };
    format!("{:02}:{:02}:{:02}", t.wHour, t.wMinute, t.wSecond)
}

pub fn now_stamp() -> String {
    let mut t: SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe { GetLocalTime(&mut t) };
    format!("{:02}.{:02} {:02}:{:02}", t.wDay, t.wMonth, t.wHour, t.wMinute)
}

impl Entry {
    pub fn new(lvl: Lvl, text: impl Into<String>) -> Self {
        Self { time: now_hms(), lvl, text: text.into() }
    }
}

impl Lvl {
    pub fn tag(self) -> &'static str {
        match self {
            Lvl::Info => "INFO",
            Lvl::Ok => "OK",
            Lvl::Err => "ERR",
        }
    }
}
