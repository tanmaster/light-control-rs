//! Background thread that owns the HID device.
//!
//! Static mode keeps the strip showing the wanted colour:
//! * re-opens and re-sends when the strip is plugged back in,
//! * re-sends after the computer wakes up (detected by a jump in wall-clock time,
//!   because the strip loses power and falls back to its rainbow mode),
//! * re-sends every 30 s as a cheap liveness check, which also catches
//!   re-enumerations that happen too fast to notice by polling.
//!
//! Sync mode streams frames from the screen-sync thread instead.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use hidapi::{HidApi, HidDevice};
use tao::event_loop::EventLoopProxy;

use crate::protocol::{self, Sequence, PRODUCT_ID, VENDOR_ID};
use crate::tray::UserEvent;

const POLL: Duration = Duration::from_secs(2);
const REFRESH: Duration = Duration::from_secs(30);

pub enum Command {
    /// Show this (already brightness-scaled) colour. Black means off. Ends sync mode.
    Show([u8; 3]),
    /// One screen-sync frame: a colour per LED, in LED order.
    Frame(Vec<[u8; 3]>),
    /// Re-open the device and send the current colour again.
    Resend,
    Quit,
}

pub fn spawn(proxy: EventLoopProxy<UserEvent>, led_count: u8, initial: [u8; 3]) -> Sender<Command> {
    let (tx, rx) = mpsc::channel();
    thread::Builder::new()
        .name("device".into())
        .spawn(move || {
            Worker {
                proxy,
                led_count,
                rgb: initial,
                frame: None,
                syncing: false,
                seq: Sequence::default(),
                api: None,
                dev: None,
                dirty: true,
                connected: None,
                last_sent: Instant::now(),
                last_check: None,
            }
            .run(rx)
        })
        .expect("failed to start device thread");
    tx
}

/// Send a single packet to the strip, outside the tray app (used by `--test-layout`).
pub fn send_once(packet: &[u8]) -> Result<(), String> {
    let api = HidApi::new().map_err(|e| format!("hidapi init failed: {e}"))?;
    let dev = api
        .open(VENDOR_ID, PRODUCT_ID)
        .map_err(|e| format!("could not open light strip: {e}"))?;
    for report in protocol::to_reports(packet) {
        dev.write(&report).map_err(|e| format!("write failed: {e}"))?;
    }
    Ok(())
}

struct Worker {
    proxy: EventLoopProxy<UserEvent>,
    led_count: u8,
    rgb: [u8; 3],
    /// Latest screen-sync frame not yet sent.
    frame: Option<Vec<[u8; 3]>>,
    syncing: bool,
    seq: Sequence,
    api: Option<HidApi>,
    dev: Option<HidDevice>,
    /// The static colour still has to be (re)sent.
    dirty: bool,
    connected: Option<bool>,
    last_sent: Instant,
    last_check: Option<Instant>,
}

impl Worker {
    fn run(mut self, rx: Receiver<Command>) {
        let mut force_open = true;
        let mut last_wall = SystemTime::now();

        loop {
            // Monotonic clocks may stop while the machine sleeps; wall-clock time does not.
            let now = SystemTime::now();
            if now.duration_since(last_wall).is_ok_and(|d| d > POLL * 4) {
                self.dev = None;
                force_open = true;
            }
            last_wall = now;

            // Presence check every POLL, even while frames are streaming in.
            if force_open || self.last_check.map_or(true, |t| t.elapsed() >= POLL) {
                self.last_check = Some(Instant::now());
                self.check_presence();
                if self.dev.is_none() {
                    self.open();
                }
            }
            force_open = false;

            if self.syncing {
                if let Some(frame) = self.frame.take() {
                    self.send_frame(&frame);
                }
            } else {
                if self.dev.is_some() && self.last_sent.elapsed() >= REFRESH {
                    self.dirty = true;
                }
                if self.dirty {
                    self.send_static();
                }
            }

            match rx.recv_timeout(POLL) {
                Ok(cmd) => {
                    if !self.handle(cmd, &mut force_open) {
                        return;
                    }
                    // Coalesce bursts (picker dragging, sync frames): only the latest matters.
                    while let Ok(cmd) = rx.try_recv() {
                        if !self.handle(cmd, &mut force_open) {
                            return;
                        }
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }

    /// Returns false when the worker should stop.
    fn handle(&mut self, cmd: Command, force_open: &mut bool) -> bool {
        match cmd {
            Command::Show(rgb) => {
                self.rgb = rgb;
                self.syncing = false;
                self.frame = None;
                self.dirty = true;
            }
            Command::Frame(colors) => {
                self.syncing = true;
                self.frame = Some(colors);
            }
            Command::Resend => {
                self.dev = None;
                self.dirty = true;
                *force_open = true;
            }
            Command::Quit => return false,
        }
        true
    }

    fn check_presence(&mut self) {
        if self.api.is_none() {
            match HidApi::new() {
                Ok(api) => self.api = Some(api),
                Err(e) => {
                    eprintln!("hidapi init failed: {e}");
                    self.set_connected(false);
                    return;
                }
            }
        }
        let Some(api) = self.api.as_mut() else { return };
        if let Err(e) = api.refresh_devices() {
            eprintln!("could not list HID devices: {e}");
            return;
        }
        let present = api
            .device_list()
            .any(|d| d.vendor_id() == VENDOR_ID && d.product_id() == PRODUCT_ID);
        if !present {
            self.dev = None;
            self.set_connected(false);
        }
    }

    fn open(&mut self) {
        let Some(api) = self.api.as_ref() else { return };
        match api.open(VENDOR_ID, PRODUCT_ID) {
            Ok(dev) => {
                self.dev = Some(dev);
                self.dirty = true;
            }
            Err(e) => {
                if self.connected != Some(false) {
                    eprintln!("could not open light strip: {e}");
                    #[cfg(target_os = "linux")]
                    eprintln!("on Linux, install 70-light-control.rules (in the download, see README) and replug the strip");
                }
                self.set_connected(false);
            }
        }
    }

    fn send_static(&mut self) {
        if self.dev.is_none() {
            return;
        }
        let packet = protocol::static_color(self.seq.next(), self.rgb, self.led_count);
        if self.write(&packet) {
            self.dirty = false;
            self.last_sent = Instant::now();
        }
    }

    fn send_frame(&mut self, colors: &[[u8; 3]]) {
        if self.dev.is_none() {
            return;
        }
        let groups = protocol::merge_groups(colors, protocol::MAX_SYNC_GROUPS);
        let packet = protocol::sync_frame(self.seq.next(), &groups);
        self.write(&packet);
    }

    /// Write a packet as one or more reports. On failure the device is dropped
    /// so the next presence check reconnects.
    fn write(&mut self, packet: &[u8]) -> bool {
        let Some(dev) = self.dev.as_ref() else { return false };
        let result = protocol::to_reports(packet)
            .iter()
            .try_for_each(|report| dev.write(report).map(|_| ()));
        match result {
            Ok(()) => {
                self.set_connected(true);
                true
            }
            Err(e) => {
                eprintln!("write failed, will reconnect: {e}");
                self.dev = None;
                // Make sure the static colour is restored after reconnecting.
                self.dirty = true;
                self.set_connected(false);
                false
            }
        }
    }

    fn set_connected(&mut self, connected: bool) {
        if self.connected != Some(connected) {
            self.connected = Some(connected);
            let _ = self.proxy.send_event(UserEvent::Connected(connected));
        }
    }
}
