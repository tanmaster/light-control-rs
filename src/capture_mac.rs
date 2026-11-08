//! macOS screen capture via ScreenCaptureKit.
//!
//! The GPU scales the screen down to a tiny frame (e.g. 160×90) before it reaches us,
//! and macOS only delivers a new frame when the screen content actually changed.
//! Compared with full-resolution screenshots this cuts memory traffic by ~1000×.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use screencapturekit::cm::SCFrameStatus;
use screencapturekit::cv::CVPixelBufferLockFlags;
use screencapturekit::prelude::*;

use crate::sync::Frame;

/// Width of the frames we ask macOS for; the height follows the screen's aspect ratio.
const OUTPUT_WIDTH: u32 = 160;
const FPS: u32 = 30;

#[derive(Default)]
struct Shared {
    latest: Mutex<Option<Frame>>,
    frames: AtomicU64,
}

pub struct MacCapture {
    stream: SCStream,
    shared: Arc<Shared>,
    seen: u64,
}

impl MacCapture {
    /// Start streaming the display with this CoreGraphics display id
    /// (falls back to the first display if it is not found).
    pub fn start(display_id: Option<u32>) -> Result<Self, String> {
        let content = SCShareableContent::get().map_err(|e| format!("cannot list displays: {e}"))?;
        let displays = content.displays();
        let display = display_id
            .and_then(|id| displays.iter().find(|d| d.display_id() == id))
            .or_else(|| displays.first())
            .ok_or("no display found")?;

        let (w, h) = (display.width().max(1), display.height().max(1));
        let out_h = ((OUTPUT_WIDTH * h) / w).max(1);

        let filter = SCContentFilter::create()
            .with_display(display)
            .with_excluding_windows(&[])
            .build()
            .map_err(|e| format!("cannot create content filter: {e}"))?;
        let config = SCStreamConfiguration::new()
            .with_width(OUTPUT_WIDTH)
            .with_height(out_h)
            .with_pixel_format(PixelFormat::BGRA)
            .with_fps(FPS)
            .with_shows_cursor(false)
            .with_queue_depth(3);

        let shared = Arc::new(Shared::default());
        let mut stream =
            SCStream::new(&filter, &config).map_err(|e| format!("cannot create stream: {e}"))?;
        {
            let shared = shared.clone();
            stream
                .add_output_handler(
                    move |sample: CMSampleBuffer, kind: SCStreamOutputType| {
                        if kind == SCStreamOutputType::Screen {
                            on_frame(&sample, &shared);
                        }
                    },
                    SCStreamOutputType::Screen,
                )
                .map_err(|e| format!("cannot attach frame handler: {e}"))?;
        }
        stream
            .start_capture()
            .map_err(|e| format!("cannot start capture: {e}"))?;

        Ok(Self { stream, shared, seen: 0 })
    }

    /// The newest frame, if one arrived since the last call.
    pub fn next_frame(&mut self) -> Option<Frame> {
        let count = self.shared.frames.load(Ordering::Acquire);
        if count == self.seen {
            return None;
        }
        self.seen = count;
        self.shared.latest.lock().ok()?.take()
    }
}

impl Drop for MacCapture {
    fn drop(&mut self) {
        let _ = self.stream.stop_capture();
    }
}

/// Runs on ScreenCaptureKit's queue: copy the small BGRA frame into a packed RGBA buffer.
fn on_frame(sample: &CMSampleBuffer, shared: &Shared) {
    // Idle frames (nothing changed on screen) carry no new image.
    if sample.frame_status().is_some_and(|s| s != SCFrameStatus::Complete) {
        return;
    }
    let Some(buffer) = sample.pixel_buffer() else { return };
    let Ok(guard) = buffer.lock(CVPixelBufferLockFlags::READ_ONLY) else { return };
    let (width, height, stride) = (guard.width(), guard.height(), guard.bytes_per_row());
    // SAFETY: the buffer stays locked for as long as `guard` lives.
    let Some(data) = (unsafe { guard.as_slice() }) else { return };

    let mut rgba = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        let start = y * stride;
        let Some(row) = data.get(start..start + width * 4) else { return };
        for px in row.chunks_exact(4) {
            rgba.extend_from_slice(&[px[2], px[1], px[0], 255]);
        }
    }
    if let Ok(mut latest) = shared.latest.lock() {
        *latest = Some(Frame { rgba, width: width as u32, height: height as u32 });
    }
    shared.frames.fetch_add(1, Ordering::Release);
}
