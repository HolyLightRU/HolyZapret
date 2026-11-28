const USAGE: &str = "использование: RayOfLight --bin <папка bin> --lists <папка списков> [--priority -100] [--log <файл>] [--app-all a.exe,b.exe] [--app-skip c.exe] [--follow <служба>] [--service] [--live <файл>] [--live-ms 2000] [--experiment general|sni-split|random|ttl|disorder|badseq|md5sig|badsum|adaptive] [--quic general|random|shuffle|block] [--autolist <файл>] [--own http,google,cf,myip-tcp,myip-quic,game-tcp,game-udp,voice] [--game-tcp 1024-65535] [--game-udp 1024-65535] [--memory <файл>] [--verbose] [--dry-run] [--forward-watch] [--config <файл настроек: по одному параметру в строке>]";

fn main() {
    rayoflight::sys::utf8_console();
    let raw: Vec<String> = std::env::args().skip(1).collect();
    if let Some(path) = rayoflight::run::config_path(&raw) {
        if raw.iter().any(|a| a == "--service") {
            let r = rayoflight::svc::dispatch(move || if rayoflight::run::run_config(path).is_ok() { 0 } else { 1 });
            if let Err(e) = r {
                eprintln!("RayOfLight: {e}");
                std::process::exit(1);
            }
            return;
        }
        if let Err(e) = rayoflight::run::run_config(path) {
            eprintln!("RayOfLight: {e}");
            std::process::exit(1);
        }
        return;
    }
    let args = match rayoflight::run::parse_args(raw) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("RayOfLight: {e}\n{USAGE}");
            std::process::exit(2);
        }
    };
    if args.service {
        let r = rayoflight::svc::dispatch(move || if rayoflight::run::run(args).is_ok() { 0 } else { 1 });
        if let Err(e) = r {
            eprintln!("RayOfLight: {e}");
            std::process::exit(1);
        }
        return;
    }
    if let Err(e) = rayoflight::run::run(args) {
        eprintln!("RayOfLight: {e}");
        std::process::exit(1);
    }
}
