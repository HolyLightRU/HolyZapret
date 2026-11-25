pub mod appcheck;
pub mod autostart;
pub mod checks;
pub mod coverage;
pub mod crash;
pub mod diagnostics;
pub mod discover;
pub mod fetch;
pub mod flowseal;
pub mod gamefilter;
pub mod gateway;
pub mod health;
pub mod hosts;
pub mod icons;
pub mod known;
pub mod lanproxy;
pub mod lists;
pub mod names;
pub mod net;
pub mod netspeed;
pub mod paths;
pub mod picker;
pub mod pickstore;
pub mod rules;
pub mod procs;
pub mod ray;
pub mod relocate;
pub mod report;
pub mod reset;
pub mod selfupdate;
pub mod service;
pub mod settings;
pub mod share;
pub mod tgproxy;
pub mod tgwarp;
pub mod strategy;
pub mod unpack;
pub mod update;
pub mod voice;
pub mod warp;

use std::os::windows::process::CommandExt;
use std::process::Command;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub const TEST_BUILD: bool = option_env!("HZ_CHANNEL").is_some();
pub const EXPERIMENTAL: &str = "experimental";

pub fn run_hidden(program: &str, args: &[&str]) -> (bool, String) {
    match Command::new(program).args(args).creation_flags(CREATE_NO_WINDOW).output() {
        Ok(o) => {
            let mut s = names::oem(&o.stdout);
            s.push_str(&names::oem(&o.stderr));
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
