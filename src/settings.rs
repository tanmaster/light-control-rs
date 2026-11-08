use std::fs;
use std::path::PathBuf;

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

use crate::color::{self, Tuning};
use crate::layout::{default_layout, Segment};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Chosen colour at full brightness.
    pub color: [u8; 3],
    /// 1..=100 percent, applied by scaling the colour.
    pub brightness: u8,
    /// Whether the light should be on.
    pub on: bool,
    /// Number of LEDs on the strip (80 for the 4-sided monitor kit).
    pub led_count: u8,
    /// Whether screen sync was running (resumed on next start).
    pub sync: bool,
    /// Screen to mirror (as reported by the OS); `None` = main screen.
    pub sync_screen: Option<String>,
    /// Screen-sync colour enhancement (see `color.rs`): 1.0 = unchanged.
    pub sync_saturation: f32,
    pub sync_gamma: f32,
    /// Where the LEDs sit around the screen, in LED order (see `layout.rs`).
    pub layout: Vec<Segment>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            color: [255, 255, 255],
            brightness: 100,
            on: true,
            led_count: 80,
            sync: false,
            sync_screen: None,
            sync_saturation: color::DEFAULT.saturation,
            sync_gamma: color::DEFAULT.gamma,
            layout: default_layout(),
        }
    }
}

impl Settings {
    fn path() -> Option<PathBuf> {
        ProjectDirs::from("com", "tanmaster", "light-control")
            .map(|d| d.config_dir().join("settings.json"))
    }

    pub fn load() -> Self {
        let Some(path) = Self::path() else {
            return Self::default();
        };
        match fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
                eprintln!("ignoring unreadable {}: {e}", path.display());
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) {
        let Some(path) = Self::path() else { return };
        let result = path
            .parent()
            .map_or(Ok(()), fs::create_dir_all)
            .and_then(|_| fs::write(&path, serde_json::to_string_pretty(self).unwrap_or_default()));
        if let Err(e) = result {
            eprintln!("could not save {}: {e}", path.display());
        }
    }

    pub fn tuning(&self) -> Tuning {
        Tuning {
            saturation: self.sync_saturation.clamp(0.0, 4.0),
            gamma: self.sync_gamma.clamp(0.2, 4.0),
        }
    }

    /// The colour that should actually be sent to the strip.
    pub fn output_rgb(&self) -> [u8; 3] {
        if !self.on {
            return [0, 0, 0];
        }
        let b = u16::from(self.brightness.clamp(1, 100));
        self.color.map(|c| ((u16::from(c) * b + 50) / 100) as u8)
    }
}
