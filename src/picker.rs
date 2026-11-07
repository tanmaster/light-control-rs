//! Colour picker window. Runs as its own short-lived process (`light-control --picker`)
//! so the always-running tray process stays tiny and free of GUI event-loop conflicts.
//!
//! It reports every change on stdout as one line: `RRGGBB <brightness 1-100>`.

use std::io::Write;

use eframe::egui::{
    self,
    color_picker::{color_picker_color32, Alpha},
    Color32,
};

pub fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 || !s.is_ascii() {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

pub fn to_hex(c: [u8; 3]) -> String {
    format!("{:02X}{:02X}{:02X}", c[0], c[1], c[2])
}

/// Parse one line written by the picker process.
pub fn parse_line(line: &str) -> Option<([u8; 3], u8)> {
    let mut parts = line.split_whitespace();
    let color = parse_hex(parts.next()?)?;
    let brightness: u8 = parts.next()?.parse().ok()?;
    Some((color, brightness.clamp(1, 100)))
}

struct Picker {
    color: Color32,
    brightness: u8,
    hex: String,
    sent: ([u8; 3], u8),
}

impl Picker {
    fn rgb(&self) -> [u8; 3] {
        [self.color.r(), self.color.g(), self.color.b()]
    }

    fn emit_if_changed(&mut self) {
        let current = (self.rgb(), self.brightness);
        if current == self.sent {
            return;
        }
        self.sent = current;
        let mut out = std::io::stdout().lock();
        let ok = writeln!(out, "{} {}", to_hex(current.0), current.1).and_then(|_| out.flush());
        if ok.is_err() {
            // The tray process is gone; nothing left to do.
            std::process::exit(0);
        }
    }
}

impl eframe::App for Picker {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.spacing_mut().slider_width = 260.0;
            if color_picker_color32(ui, &mut self.color, Alpha::Opaque) {
                self.hex = to_hex(self.rgb());
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label("Hex #");
                if ui.text_edit_singleline(&mut self.hex).changed() {
                    if let Some([r, g, b]) = parse_hex(&self.hex) {
                        self.color = Color32::from_rgb(r, g, b);
                    }
                }
            });
            ui.add(egui::Slider::new(&mut self.brightness, 1..=100).text("Brightness %"));
        });
        self.emit_if_changed();
    }
}

pub fn run(color: [u8; 3], brightness: u8) -> Result<(), eframe::Error> {
    let brightness = brightness.clamp(1, 100);
    let app = Picker {
        color: Color32::from_rgb(color[0], color[1], color[2]),
        brightness,
        hex: to_hex(color),
        sent: (color, brightness),
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("light-control")
            .with_inner_size([300.0, 400.0])
            .with_resizable(false),
        ..Default::default()
    };
    eframe::run_native("light-control", options, Box::new(|_cc| Ok(Box::new(app))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_roundtrip() {
        assert_eq!(parse_hex("#1A2b3C"), Some([0x1a, 0x2b, 0x3c]));
        assert_eq!(to_hex([0x1a, 0x2b, 0x3c]), "1A2B3C");
        assert_eq!(parse_hex("12345"), None);
        assert_eq!(parse_hex("zzzzzz"), None);
    }

    #[test]
    fn line_parsing() {
        assert_eq!(parse_line("FF8000 42"), Some(([255, 128, 0], 42)));
        assert_eq!(parse_line("FF8000 0"), Some(([255, 128, 0], 1)));
        assert_eq!(parse_line("FF8000"), None);
    }
}
