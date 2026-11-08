//! Screen sync: capture a screen, sample the edges, stream colours to the strip.
//!
//! Capture backends:
//! * macOS: ScreenCaptureKit (`capture_mac.rs`), GPU-scaled tiny frames, only on change.
//! * everywhere else, and as a fallback: full screenshots via `xcap`.

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use xcap::Monitor;

use crate::color::{self, Tuning};
use crate::device::Command;
use crate::layout::{self, Segment};

/// Loop interval (~30 fps).
const FRAME: Duration = Duration::from_millis(33);
/// Weight of the newest frame. Higher = snappier, lower = smoother.
/// The official app averages the last 10 frames, which is what makes it feel laggy.
const NEW_FRAME_WEIGHT: f32 = 0.6;
/// How often to print timing stats to stderr.
const STATS_EVERY: Duration = Duration::from_secs(5);

/// A captured frame as tightly packed RGBA.
pub struct Frame {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

pub struct SyncHandle {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl SyncHandle {
    pub fn stop(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// A screen as offered in the tray menu.
pub struct ScreenInfo {
    /// Stable identifier stored in the settings (`Monitor::name`).
    pub name: String,
    /// Human-readable label for the menu.
    pub label: String,
}

/// All connected screens, main screen first.
pub fn screens() -> Vec<ScreenInfo> {
    let mut list: Vec<(bool, ScreenInfo)> = Monitor::all()
        .unwrap_or_default()
        .iter()
        .filter_map(|m| {
            let name = m.name().ok()?;
            let primary = m.is_primary().unwrap_or(false);
            let mut label = m.friendly_name().unwrap_or_else(|_| name.clone());
            if primary {
                label.push_str(" (main)");
            }
            Some((primary, ScreenInfo { name, label }))
        })
        .collect();
    list.sort_by_key(|(primary, _)| !*primary);
    list.into_iter().map(|(_, s)| s).collect()
}

/// The screen to capture: the one with `name`, or the main screen if it is not connected.
fn find_monitor(name: Option<&str>) -> Option<Monitor> {
    let all = Monitor::all().unwrap_or_default();
    let wanted = name.and_then(|n| all.iter().position(|m| m.name().ok().as_deref() == Some(n)));
    let index = wanted.or_else(|| all.iter().position(|m| m.is_primary().unwrap_or(false)));
    match index {
        Some(i) => all.into_iter().nth(i),
        // The primary screen contains the global origin on macOS, Windows and X11.
        None => Monitor::from_point(0, 0).ok(),
    }
}

enum Source {
    #[cfg(target_os = "macos")]
    Native(crate::capture_mac::MacCapture),
    Screenshots(Monitor),
}

impl Source {
    /// Open the best available capture method. Returns the source and a label for logging.
    fn open(screen: Option<&str>) -> Result<(Self, String), String> {
        let monitor = find_monitor(screen).ok_or("no monitor found")?;
        let label = monitor
            .friendly_name()
            .or_else(|_| monitor.name())
            .unwrap_or_default();

        #[cfg(target_os = "macos")]
        {
            match crate::capture_mac::MacCapture::start(monitor.id().ok()) {
                Ok(capture) => {
                    return Ok((Self::Native(capture), format!("{label} via ScreenCaptureKit")))
                }
                Err(e) => eprintln!("screen sync: ScreenCaptureKit unavailable ({e}), using screenshots"),
            }
        }

        Ok((Self::Screenshots(monitor), format!("{label} via screenshots")))
    }

    /// `Ok(None)` means no new frame since the last call (the screen did not change).
    fn next_frame(&mut self) -> Result<Option<Frame>, String> {
        match self {
            #[cfg(target_os = "macos")]
            Self::Native(capture) => Ok(capture.next_frame()),
            Self::Screenshots(monitor) => monitor
                .capture_image()
                .map(|img| {
                    let (width, height) = (img.width(), img.height());
                    Some(Frame { rgba: img.into_raw(), width, height })
                })
                .map_err(|e| e.to_string()),
        }
    }
}

pub fn start(
    device: Sender<Command>,
    layout: Vec<Segment>,
    brightness: Arc<AtomicU8>,
    screen: Option<String>,
    tuning: Tuning,
) -> SyncHandle {
    let stop = Arc::new(AtomicBool::new(false));
    let thread = {
        let stop = stop.clone();
        thread::Builder::new()
            .name("screen-sync".into())
            .spawn(move || run(&device, &layout, &brightness, screen.as_deref(), tuning, &stop))
            .expect("failed to start screen sync thread")
    };
    SyncHandle { stop, thread: Some(thread) }
}

fn run(
    device: &Sender<Command>,
    layout: &[Segment],
    brightness: &AtomicU8,
    screen: Option<&str>,
    tuning: Tuning,
    stop: &AtomicBool,
) {
    let mut source: Option<Source> = None;
    let mut label = String::new();
    let mut zones: Vec<layout::Rect> = Vec::new();
    let mut zones_for = (0u32, 0u32);
    let mut colors: Vec<[u8; 3]> = Vec::new();
    let mut smooth: Vec<[f32; 3]> = Vec::new();
    let mut last_out: Vec<[u8; 3]> = Vec::new();
    let mut reported_error = false;

    let mut stats_start = Instant::now();
    let mut new_frames = 0u32;
    let mut updates = 0u32;
    let mut capture_time = Duration::ZERO;

    while !stop.load(Ordering::Relaxed) {
        let tick = Instant::now();

        if source.is_none() {
            match Source::open(screen) {
                Ok((s, l)) => {
                    eprintln!("screen sync: capturing {l}");
                    source = Some(s);
                    label = l;
                }
                Err(e) => {
                    if !reported_error {
                        eprintln!("screen sync: {e}");
                        reported_error = true;
                    }
                    thread::sleep(Duration::from_secs(1));
                    continue;
                }
            }
        }

        match source.as_mut().map(Source::next_frame) {
            Some(Ok(Some(frame))) => {
                reported_error = false;
                capture_time += tick.elapsed();
                new_frames += 1;
                if zones_for != (frame.width, frame.height) {
                    zones = layout::zones(layout, frame.width, frame.height);
                    zones_for = (frame.width, frame.height);
                }
                colors = layout::sample(&frame.rgba, frame.width, frame.height, &zones);
            }
            Some(Ok(None)) | None => {} // nothing new on screen; keep easing towards the last colours
            Some(Err(e)) => {
                if !reported_error {
                    eprintln!("screen sync: capture failed: {e}");
                    #[cfg(target_os = "macos")]
                    eprintln!("screen sync: allow Screen Recording for this app (or your terminal) in System Settings → Privacy & Security");
                    reported_error = true;
                }
                source = None;
                thread::sleep(Duration::from_secs(1));
                continue;
            }
        }

        if !colors.is_empty() {
            if smooth.len() != colors.len() {
                smooth = colors.iter().map(|c| c.map(f32::from)).collect();
            }
            let scale = f32::from(brightness.load(Ordering::Relaxed).clamp(1, 100)) / 100.0;
            let out: Vec<[u8; 3]> = smooth
                .iter_mut()
                .zip(&colors)
                .map(|(s, c)| {
                    for k in 0..3 {
                        s[k] += (f32::from(c[k]) - s[k]) * NEW_FRAME_WEIGHT;
                    }
                    color::enhance(*s, tuning).map(|v| (v * scale).round().clamp(0.0, 255.0) as u8)
                })
                .collect();

            // Only talk to the strip when something visibly changed.
            if out != last_out {
                if device.send(Command::Frame(out.clone())).is_err() {
                    return; // device thread is gone
                }
                last_out = out;
                updates += 1;
            }
        }

        if stats_start.elapsed() >= STATS_EVERY {
            let secs = stats_start.elapsed().as_secs_f32();
            eprintln!(
                "screen sync: {:.1} new frames/s, {:.1} strip updates/s, {:.2} ms/frame ({label}, {}x{})",
                new_frames as f32 / secs,
                updates as f32 / secs,
                capture_time.as_secs_f32() * 1000.0 / new_frames.max(1) as f32,
                zones_for.0,
                zones_for.1,
            );
            stats_start = Instant::now();
            new_frames = 0;
            updates = 0;
            capture_time = Duration::ZERO;
        }

        if let Some(rest) = FRAME.checked_sub(tick.elapsed()) {
            thread::sleep(rest);
        }
    }
}
