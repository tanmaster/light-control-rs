//! Background thread that owns the HID device.
//!
//! It keeps the strip showing the wanted colour:
//! * re-opens and re-sends when the strip is plugged back in,
//! * re-sends after the computer wakes up (detected by a jump in wall-clock time,
//!   because the strip loses power and falls back to its rainbow mode),
//! * re-sends every 30 s as a cheap liveness check, which also catches
//!   re-enumerations that happen too fast to notice by polling.

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
    /// Show this (already brightness-scaled) colour. Black means off.
    Show([u8; 3]),
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
                seq: Sequence::default(),
                api: None,
                dev: None,
                dirty: true,
                connected: None,
                last_sent: Instant::now(),
            }
            .run(rx)
        })
        .expect("failed to start device thread");
    tx
}

struct Worker {
    proxy: EventLoopProxy<UserEvent>,
    led_count: u8,
    rgb: [u8; 3],
    seq: Sequence,
    api: Option<HidApi>,
    dev: Option<HidDevice>,
    /// The colour still has to be (re)sent.
    dirty: bool,
    connected: Option<bool>,
    last_sent: Instant,
}

impl Worker {
    fn run(mut self, rx: Receiver<Command>) {
        let mut tick = true;
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

            if tick {
                self.check_presence();
            }
            if self.dev.is_none() && (tick || force_open) {
                self.open();
            }
            force_open = false;
            if self.dev.is_some() && self.last_sent.elapsed() >= REFRESH {
                self.dirty = true;
            }
            if self.dirty {
                self.send();
            }

            tick = false;
            match rx.recv_timeout(POLL) {
                Ok(cmd) => {
                    if !self.handle(cmd, &mut force_open) {
                        return;
                    }
                    // Coalesce bursts (e.g. dragging in the colour picker): only the latest matters.
                    while let Ok(cmd) = rx.try_recv() {
                        if !self.handle(cmd, &mut force_open) {
                            return;
                        }
                    }
                }
                Err(RecvTimeoutError::Timeout) => tick = true,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }

    /// Returns false when the worker should stop.
    fn handle(&mut self, cmd: Command, force_open: &mut bool) -> bool {
        match cmd {
            Command::Show(rgb) => {
                self.rgb = rgb;
                self.dirty = true;
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
                    eprintln!("on Linux, install the udev rule from packaging/linux (see README)");
                }
                self.set_connected(false);
            }
        }
    }

    fn send(&mut self) {
        if self.dev.is_none() {
            return;
        }
        let packet = protocol::static_color(self.seq.next(), self.rgb, self.led_count);
        let report = protocol::to_report(&packet);
        let result = self.dev.as_ref().map(|d| d.write(&report));
        match result {
            Some(Ok(_)) => {
                self.dirty = false;
                self.last_sent = Instant::now();
                self.set_connected(true);
            }
            Some(Err(e)) => {
                eprintln!("write failed, will reconnect: {e}");
                self.dev = None;
                self.set_connected(false);
            }
            None => {}
        }
    }

    fn set_connected(&mut self, connected: bool) {
        if self.connected != Some(connected) {
            self.connected = Some(connected);
            let _ = self.proxy.send_event(UserEvent::Connected(connected));
        }
    }
}
