//! The always-running part: tray icon + menu, owns settings, talks to the device thread.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command as Process, Stdio};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;

use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tray_icon::menu::{
    CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu,
};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

use crate::color::{self, Tuning};
use crate::device::{self, Command};
use crate::settings::Settings;
use crate::sync::{self, SyncHandle};
use crate::{autostart, picker};

pub enum UserEvent {
    Menu(MenuId),
    Picker([u8; 3], u8),
    PickerClosed,
    Connected(bool),
}

/// Everything the event loop owns besides the menu items.
struct App {
    settings: Settings,
    device: Sender<Command>,
    brightness: Arc<AtomicU8>,
    sync: Option<SyncHandle>,
    tray: Option<TrayIcon>,
    child: Option<Child>,
}

impl App {
    /// Push the current static colour, unless screen sync owns the strip.
    fn apply(&self) {
        if self.sync.is_none() {
            let _ = self.device.send(Command::Show(self.settings.output_rgb()));
        }
    }

    fn update_icon(&self) {
        if let Some(t) = &self.tray {
            let _ = t.set_icon(Some(icon(self.settings.color, self.settings.on)));
        }
    }

    fn start_sync(&mut self) {
        if self.sync.is_none() {
            self.sync = Some(sync::start(
                self.device.clone(),
                self.settings.layout.clone(),
                self.brightness.clone(),
                self.settings.sync_screen.clone(),
                self.settings.tuning(),
            ));
        }
    }

    /// Restart a running sync so it picks up a changed setting.
    fn restart_sync(&mut self) {
        if let Some(handle) = self.sync.take() {
            handle.stop();
            self.start_sync();
        }
    }

    fn stop_sync(&mut self) {
        if let Some(handle) = self.sync.take() {
            handle.stop();
        }
        self.apply();
    }
}

pub fn run() {
    let settings = Settings::load();

    #[allow(unused_mut)]
    let mut event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    #[cfg(target_os = "macos")]
    {
        // Menu-bar only: no Dock icon, no app switcher entry.
        use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
        event_loop.set_activation_policy(ActivationPolicy::Accessory);
    }
    let proxy = event_loop.create_proxy();
    {
        let proxy = proxy.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let _ = proxy.send_event(UserEvent::Menu(event.id));
        }));
    }

    let status = MenuItem::new("Strip: searching…", false, None);
    let open = MenuItem::new("Open colour picker…", true, None);
    let on = CheckMenuItem::new("Light on", true, settings.on, None);
    let screen = CheckMenuItem::new("Screen sync", true, settings.sync && settings.on, None);
    // "Sync screen" submenu: one entry per connected screen, used like radio buttons.
    // `None` = follow the main screen.
    let screen_menu = Submenu::new("Sync screen", true);
    let mut screen_items: Vec<(Option<String>, CheckMenuItem)> = Vec::new();
    {
        let mut choices: Vec<(Option<String>, String)> = vec![(None, "Main screen".to_string())];
        choices.extend(sync::screens().into_iter().map(|s| (Some(s.name), s.label)));
        let selected_known = choices.iter().any(|(n, _)| *n == settings.sync_screen);
        for (name, label) in choices {
            let checked = if selected_known {
                name == settings.sync_screen
            } else {
                name.is_none()
            };
            let item = CheckMenuItem::new(label, true, checked, None);
            let _ = screen_menu.append(&item);
            screen_items.push((name, item));
        }
    }
    // "Colours" submenu: screen-sync colour enhancement presets, used like radio buttons.
    let colours_menu = Submenu::new("Colours", true);
    let colour_items: Vec<(Tuning, CheckMenuItem)> = color::PRESETS
        .iter()
        .map(|&(label, tuning)| {
            let item = CheckMenuItem::new(label, true, tuning == settings.tuning(), None);
            let _ = colours_menu.append(&item);
            (tuning, item)
        })
        .collect();
    let resend = MenuItem::new("Resend colour", true, None);
    let login = CheckMenuItem::new("Launch at login", true, autostart::is_enabled(), None);
    let quit = MenuItem::new("Quit", true, None);

    let menu = Menu::new();
    menu.append_items(&[
        &status,
        &PredefinedMenuItem::separator(),
        &open,
        &on,
        &screen,
        &screen_menu,
        &colours_menu,
        &resend,
        &PredefinedMenuItem::separator(),
        &login,
        &PredefinedMenuItem::separator(),
        &quit,
    ])
    .expect("failed to build tray menu");

    let device = device::spawn(proxy.clone(), settings.led_count, settings.output_rgb());
    let mut app = App {
        brightness: Arc::new(AtomicU8::new(settings.brightness)),
        settings,
        device,
        sync: None,
        tray: None,
        child: None,
    };
    let mut menu = Some(menu);

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        match event {
            // The tray icon must be created once the event loop runs (required on macOS).
            Event::NewEvents(StartCause::Init) => {
                if let Some(menu) = menu.take() {
                    match TrayIconBuilder::new()
                        .with_menu(Box::new(menu))
                        .with_tooltip("light-control")
                        .with_icon(icon(app.settings.color, app.settings.on))
                        .build()
                    {
                        Ok(t) => app.tray = Some(t),
                        Err(e) => {
                            eprintln!("could not create tray icon: {e}");
                            *control_flow = ControlFlow::Exit;
                        }
                    }
                }
                if app.settings.sync && app.settings.on {
                    app.start_sync();
                }
            }

            Event::UserEvent(UserEvent::Menu(id)) => {
                if id == open.id() {
                    open_picker(&mut app.child, &app.settings, &proxy);
                } else if id == on.id() {
                    app.settings.on = on.is_checked();
                    if !app.settings.on && app.sync.is_some() {
                        screen.set_checked(false);
                        app.settings.sync = false;
                        app.stop_sync(); // also sends black via apply()
                    } else {
                        app.apply();
                    }
                    app.update_icon();
                    app.settings.save();
                } else if id == screen.id() {
                    app.settings.sync = screen.is_checked();
                    if app.settings.sync {
                        if !app.settings.on {
                            app.settings.on = true;
                            on.set_checked(true);
                            app.update_icon();
                        }
                        app.start_sync();
                    } else {
                        app.stop_sync();
                    }
                    app.settings.save();
                } else if let Some(pos) = screen_items.iter().position(|(_, item)| id == item.id()) {
                    // Radio behaviour: exactly one screen is checked.
                    for (i, (_, item)) in screen_items.iter().enumerate() {
                        item.set_checked(i == pos);
                    }
                    let chosen = screen_items[pos].0.clone();
                    if chosen != app.settings.sync_screen {
                        app.settings.sync_screen = chosen;
                        app.settings.save();
                        app.restart_sync();
                    }
                } else if let Some(pos) = colour_items.iter().position(|(_, item)| id == item.id()) {
                    for (i, (_, item)) in colour_items.iter().enumerate() {
                        item.set_checked(i == pos);
                    }
                    let tuning = colour_items[pos].0;
                    app.settings.sync_saturation = tuning.saturation;
                    app.settings.sync_gamma = tuning.gamma;
                    app.settings.save();
                    app.restart_sync();
                } else if id == resend.id() {
                    let _ = app.device.send(Command::Resend);
                } else if id == login.id() {
                    let want = login.is_checked();
                    if let Err(e) = autostart::set_enabled(want) {
                        eprintln!("could not change launch at login: {e}");
                        login.set_checked(!want);
                    }
                } else if id == quit.id() {
                    if let Some(mut c) = app.child.take() {
                        let _ = c.kill();
                    }
                    if let Some(handle) = app.sync.take() {
                        handle.stop();
                    }
                    app.settings.save();
                    // The strip keeps its last colour after we exit.
                    let _ = app.device.send(Command::Quit);
                    app.tray.take();
                    *control_flow = ControlFlow::Exit;
                }
            }

            Event::UserEvent(UserEvent::Picker(color, brightness)) => {
                app.settings.color = color;
                app.settings.brightness = brightness;
                app.brightness.store(brightness, Ordering::Relaxed);
                if !app.settings.on {
                    app.settings.on = true;
                    on.set_checked(true);
                }
                // Icon and settings file are updated when the picker closes,
                // not on every drag step.
                app.apply();
            }

            Event::UserEvent(UserEvent::PickerClosed) => {
                if let Some(mut c) = app.child.take() {
                    let _ = c.wait();
                }
                app.update_icon();
                app.settings.save();
            }

            Event::UserEvent(UserEvent::Connected(connected)) => {
                status.set_text(if connected {
                    "Strip: connected"
                } else {
                    "Strip: not found"
                });
            }

            _ => {}
        }
    });
}

fn open_picker(child: &mut Option<Child>, settings: &Settings, proxy: &EventLoopProxy<UserEvent>) {
    if let Some(c) = child {
        if matches!(c.try_wait(), Ok(None)) {
            return; // already open
        }
    }
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("cannot locate own executable: {e}");
            return;
        }
    };
    let spawned = Process::new(exe)
        .arg("--picker")
        .arg(picker::to_hex(settings.color))
        .arg(settings.brightness.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .spawn();
    let mut c = match spawned {
        Ok(c) => c,
        Err(e) => {
            eprintln!("could not start colour picker: {e}");
            return;
        }
    };
    if let Some(stdout) = c.stdout.take() {
        let proxy = proxy.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Some((color, brightness)) = picker::parse_line(&line) {
                    let _ = proxy.send_event(UserEvent::Picker(color, brightness));
                }
            }
            let _ = proxy.send_event(UserEvent::PickerClosed);
        });
    }
    *child = Some(c);
}

/// A filled circle in the chosen colour, or an empty ring when the light is off.
fn icon(rgb: [u8; 3], on: bool) -> Icon {
    const SIZE: u32 = 32;
    let centre = (SIZE as f32 - 1.0) / 2.0;
    let mut rgba = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let d = ((x as f32 - centre).powi(2) + (y as f32 - centre).powi(2)).sqrt();
            let px = if d > 15.0 {
                [0, 0, 0, 0]
            } else if d > 13.0 {
                [60, 60, 60, 255]
            } else if on {
                [rgb[0], rgb[1], rgb[2], 255]
            } else if d > 10.5 {
                [150, 150, 150, 255]
            } else {
                [0, 0, 0, 0]
            };
            rgba.extend_from_slice(&px);
        }
    }
    Icon::from_rgba(rgba, SIZE, SIZE).expect("valid icon buffer")
}
