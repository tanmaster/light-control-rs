// No console window on Windows release builds.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod autostart;
#[cfg(target_os = "linux")]
mod capture_linux;
#[cfg(target_os = "macos")]
mod capture_mac;
mod color;
mod device;
mod layout;
mod picker;
mod protocol;
mod settings;
mod sync;
mod tray;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("--picker") => {
            let color = args
                .get(2)
                .and_then(|s| picker::parse_hex(s))
                .unwrap_or([255, 255, 255]);
            let brightness = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(100);
            if let Err(e) = picker::run(color, brightness) {
                eprintln!("colour picker failed: {e}");
                std::process::exit(1);
            }
        }
        Some("--test-layout") => test_layout(),
        _ => tray::run(),
    }
}

/// Light fixed LED ranges in distinct colours so the physical layout can be read off the strip.
/// Quit the tray app first, otherwise it overwrites the pattern within 30 s.
fn test_layout() {
    let groups: [protocol::Group; 5] = [
        (1, [255, 255, 0], 3),     // yellow: LEDs 1-3 (start of the strip)
        (4, [255, 0, 0], 14),      // red:    LEDs 4-14
        (15, [0, 255, 0], 40),     // green:  LEDs 15-40
        (41, [0, 0, 255], 54),     // blue:   LEDs 41-54
        (55, [255, 255, 255], 80), // white:  LEDs 55-80
    ];
    let packet = protocol::section_led(1, &groups);
    match device::send_once(&packet) {
        Ok(()) => println!(
            "Layout test sent. Looking at the screen from the front, note:\n\
             - where the YELLOW LEDs are (LEDs 1-3, the start of the strip)\n\
             - which edge(s) RED (4-14), GREEN (15-40), BLUE (41-54) and WHITE (55-80) cover\n\
             Replug the strip or start light-control to restore your colour."
        ),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
