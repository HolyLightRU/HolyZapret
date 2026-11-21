pub mod autostart;
pub mod checks;
pub mod diagnostics;
pub mod discover;
pub mod flowseal;
pub mod gamefilter;
pub mod hosts;
pub mod lists;
pub mod net;
pub mod paths;
pub mod picker;
pub mod pickstore;
pub mod procs;
pub mod selfupdate;
pub mod service;
pub mod settings;
pub mod strategy;
pub mod update;
pub mod warp;

use std::os::windows::process::CommandExt;
use std::process::Command;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub fn run_hidden(program: &str, args: &[&str]) -> (bool, String) {
    match Command::new(program).args(args).creation_flags(CREATE_NO_WINDOW).output() {
        Ok(o) => {
            let mut s = String::from_utf8_lossy(&o.stdout).into_owned();
            s.push_str(&String::from_utf8_lossy(&o.stderr));
            (o.status.success(), s)
        }
        Err(e) => (false, e.to_string()),
    }
}

pub fn process_running(image: &str) -> bool {
    procs::running(image)
}

pub fn kill_process(image: &str) -> bool {
    run_hidden("taskkill", &["/IM", image, "/F"]).0
}
