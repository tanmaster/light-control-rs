# light-control (Rust)

A small, open-source replacement for the DX-Light app, for the generic USB monitor
backlight strip sold under many names (USB `1a86:fe07`, 80 LEDs).

It sits in the tray / menu bar and keeps the strip on **one static colour of your
choice**, instead of the built-in rainbow mode, or mirrors the edges of your screen.

- colour picker (wheel, hex, brightness)
- light on/off
- screen sync (ambient light from the screen edges, ~30 fps)
- re-applies the colour automatically when the strip is plugged in, the computer
  wakes up, or the strip resets for any other reason
- remembers the colour; optional launch at login
- macOS, Windows, Linux (Ubuntu); a single native binary, no runtime needed

The successor of the Python [light-control](https://github.com/tanmaster/light-control).
The protocol is documented in [`docs/protocol.md`](docs/protocol.md).

## Build

Install Rust from <https://rustup.rs>, then:

```sh
cargo test
cargo build --release
./target/release/light-control        # Windows: target\release\light-control.exe
```

### Extra steps on Linux (Ubuntu)

Build dependencies:

```sh
sudo apt install build-essential pkg-config clang libgtk-3-dev libxdo-dev \
  libayatana-appindicator3-dev libudev-dev libxkbcommon-dev \
  libxcb1-dev libxrandr-dev libdbus-1-dev libpipewire-0.3-dev libwayland-dev libegl-dev
```

Allow your user to access the strip (otherwise only root can):

```sh
sudo cp packaging/linux/99-light-control.rules /etc/udev/rules.d/
sudo udevadm control --reload && sudo udevadm trigger
```

Then unplug and replug the strip. On GNOME the tray icon needs the AppIndicator
extension, which Ubuntu enables by default.

## Use

Click the tray icon:

- **Open colour picker…**: opens a small window; changes apply live and are saved when you close it
- **Light on**: toggle the strip off and on
- **Screen sync**: mirror the screen edges; turning it off returns to your colour.
  The picker's brightness slider also dims screen sync.
- **Sync screen ›**: which screen to mirror (main screen by default). The list is read
  when the app starts; if the chosen screen is disconnected, the main screen is used.
- **Resend colour**: force a reconnect if the strip ever shows the rainbow again
- **Launch at login**: start automatically (enable it from the final location of
  the binary, not from `target/debug`)
- **Quit**: the strip keeps its last colour

Settings live in `settings.json` in the OS config directory
(`~/Library/Application Support/com.tanmaster.light-control/` on macOS,
`~/.config/light-control/` on Linux, `%APPDATA%\tanmaster\light-control\config\` on Windows).

## Screen sync

**Permissions.** macOS asks for *Screen Recording* permission the first time
(System Settings → Privacy & Security). When started from a terminal, the
permission belongs to the terminal app. On Ubuntu with Wayland, the system asks
which screen to share. X11 and Windows need nothing.

**LED layout.** The `layout` entry in `settings.json` lists the strip's segments
in LED order, each along one screen edge, as seen from the front:

```json
"layout": [
  { "edge": "left",   "count": 14, "reverse": false },
  { "edge": "top",    "count": 26, "reverse": false },
  { "edge": "right",  "count": 14, "reverse": false },
  { "edge": "bottom", "count": 26, "reverse": false }
]
```

`reverse: false` means clockwise: top left→right, right top→bottom,
bottom right→left, left bottom→top.

To find your strip's layout, quit the tray app and run

```sh
./target/release/light-control --test-layout
```

LEDs 1–3 turn yellow, 4–14 red, 15–40 green, 41–54 blue, 55–80 white.

**Performance.** While syncing, the app prints its frame rate and capture time
every 5 seconds to the terminal.

## How it works

- `src/protocol.rs`: packet formats (`"RB"` commands and `"SC"` sync frames), unit-tested against real USB captures
- `src/device.rs`: background thread that owns the USB HID device, re-sends the colour whenever needed and streams sync frames
- `src/layout.rs`: LED positions around the screen and edge sampling
- `src/sync.rs`: screen capture loop (via [`xcap`](https://crates.io/crates/xcap)) with light smoothing
- `src/tray.rs`: tray icon and menu
- `src/picker.rs`: the colour picker window, started as a separate short-lived process (`light-control --picker`) so the always-running part stays small
- `src/autostart.rs`: LaunchAgent (macOS), XDG autostart entry (Linux), `HKCU\…\Run` (Windows)

## Disclaimer

Only tested on the hardware described above. Sending raw commands to USB devices is at your own risk.
