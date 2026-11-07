//! The always-running part: tray icon + menu, owns settings, talks to the device thread.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command as Process, Stdio};
use std::sync::mpsc::Sender;

use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

use crate::device::{self, Command};
use crate::settings::Settings;
use crate::{autostart, picker};

pub enum UserEvent {
    Menu(MenuId),
    Picker([u8; 3], u8),
    PickerClosed,
    Connected(bool),
}

pub fn run() {
    let mut settings = Settings::load();

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
    let resend = MenuItem::new("Resend colour", true, None);
    let login = CheckMenuItem::new("Launch at login", true, autostart::is_enabled(), None);
    let quit = MenuItem::new("Quit", true, None);

    let menu = Menu::new();
    menu.append_items(&[
        &status,
        &PredefinedMenuItem::separator(),
        &open,
        &on,
        &resend,
        &PredefinedMenuItem::separator(),
        &login,
        &PredefinedMenuItem::separator(),
        &quit,
    ])
    .expect("failed to build tray menu");

    let device = device::spawn(proxy.clone(), settings.led_count, settings.output_rgb());
    let mut menu = Some(menu);
    let mut tray: Option<TrayIcon> = None;
    let mut child: Option<Child> = None;

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        match event {
            // The tray icon must be created once the event loop runs (required on macOS).
            Event::NewEvents(StartCause::Init) => {
                if let Some(menu) = menu.take() {
                    match TrayIconBuilder::new()
                        .with_menu(Box::new(menu))
                        .with_tooltip("light-control")
                        .with_icon(icon(settings.color, settings.on))
                        .build()
                    {
                        Ok(t) => tray = Some(t),
                        Err(e) => {
                            eprintln!("could not create tray icon: {e}");
                            *control_flow = ControlFlow::Exit;
                        }
                    }
                }
            }

            Event::UserEvent(UserEvent::Menu(id)) => {
                if &id == open.id() {
                    open_picker(&mut child, &settings, &proxy);
                } else if &id == on.id() {
                    settings.on = on.is_checked();
                    apply(&settings, &device);
                    update_icon(&tray, &settings);
                    settings.save();
                } else if &id == resend.id() {
                    let _ = device.send(Command::Resend);
                } else if &id == login.id() {
                    let want = login.is_checked();
                    if let Err(e) = autostart::set_enabled(want) {
                        eprintln!("could not change launch at login: {e}");
                        login.set_checked(!want);
                    }
                } else if &id == quit.id() {
                    if let Some(mut c) = child.take() {
                        let _ = c.kill();
                    }
                    settings.save();
                    // The strip keeps its colour after we exit.
                    let _ = device.send(Command::Quit);
                    tray.take();
                    *control_flow = ControlFlow::Exit;
                }
            }

            Event::UserEvent(UserEvent::Picker(color, brightness)) => {
                settings.color = color;
                settings.brightness = brightness;
                if !settings.on {
                    settings.on = true;
                    on.set_checked(true);
                }
                // Icon and settings file are updated when the picker closes,
                // not on every drag step.
                apply(&settings, &device);
            }

            Event::UserEvent(UserEvent::PickerClosed) => {
                if let Some(mut c) = child.take() {
                    let _ = c.wait();
                }
                update_icon(&tray, &settings);
                settings.save();
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

fn apply(settings: &Settings, device: &Sender<Command>) {
    let _ = device.send(Command::Show(settings.output_rgb()));
}

fn update_icon(tray: &Option<TrayIcon>, settings: &Settings) {
    if let Some(t) = tray {
        let _ = t.set_icon(Some(icon(settings.color, settings.on)));
    }
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
