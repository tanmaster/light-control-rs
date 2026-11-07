// No console window on Windows release builds.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod autostart;
mod device;
mod picker;
mod protocol;
mod settings;
mod tray;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--picker") {
        let color = args
            .get(2)
            .and_then(|s| picker::parse_hex(s))
            .unwrap_or([255, 255, 255]);
        let brightness = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(100);
        if let Err(e) = picker::run(color, brightness) {
            eprintln!("colour picker failed: {e}");
            std::process::exit(1);
        }
        return;
    }
    tray::run();
}
