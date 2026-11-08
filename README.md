# light-control

[![build](https://github.com/tanmaster/light-control-rs/actions/workflows/build.yml/badge.svg)](https://github.com/tanmaster/light-control-rs/actions/workflows/build.yml)

A small, open-source replacement for the closed-source **DX-Light / Robobloq** app, for the
generic USB monitor backlight strip sold under many names (USB ID `1a86:fe07`, 80 LEDs
around the back of the monitor).

It sits in the tray / menu bar and keeps the strip on **one colour of your choice**, instead of
the built-in rainbow mode, or mirrors the edges of your screen (ambient light).
A single native binary for macOS, Windows and Linux, no runtime or account needed.

## Features

- **Static colour** with a colour picker (wheel, hex, brightness); changes apply live
- **Screen sync**: the LEDs follow the colours at the edges of your screen, ~30 fps,
  with smoothing and three colour presets (Natural / Vivid / Intense)
- **Choose which screen** to mirror on multi-monitor setups
- **Self-healing**: re-applies your colour when the strip is replugged, when the computer
  wakes up, or whenever the strip falls back to its own rainbow mode
- **Light on/off** and optional **launch at login**
- Light on resources: ~4 MB binary; on macOS screen sync uses ScreenCaptureKit and
  needs about 1% CPU on a still screen

## Download

Get the latest build from the [**Releases**](https://github.com/tanmaster/light-control-rs/releases) page:

| OS | File |
|----|------|
| macOS (Apple Silicon) | `light-control-<version>-macos-arm64.tar.gz` |
| Windows (x86-64) | `light-control-<version>-windows-x86_64.zip` |
| Linux (x86-64) | `light-control-<version>-linux-x86_64.tar.gz` |

Unpack it and run `light-control` (`light-control.exe` on Windows). Intel Macs and other
architectures can [build from source](#build-from-source).

The binaries are not code-signed, so the first start needs one extra step:

- **macOS**: if macOS says the app "cannot be verified", remove the download quarantine flag:
  `xattr -d com.apple.quarantine ./light-control`
- **Windows**: if SmartScreen appears, click *More info* → *Run anyway*.

## Setup per OS

### macOS

Screen sync needs the **Screen Recording** permission. macOS asks the first time you turn on
screen sync; you can also grant it in *System Settings → Privacy & Security → Screen Recording*.
When you start `light-control` from a terminal, the permission belongs to the terminal app.
Static colours need no permission.

### Linux (tested on Ubuntu)

Allow your user to talk to the strip (otherwise only root can). The rule is included in the
Linux download and in [`packaging/linux`](packaging/linux/99-light-control.rules):

```sh
sudo cp 99-light-control.rules /etc/udev/rules.d/
sudo udevadm control --reload && sudo udevadm trigger
```

Then unplug and replug the strip.

The tray icon uses **AppIndicator**. Ubuntu enables it by default; on other GNOME
distributions install the *AppIndicator and KStatusNotifierItem Support* extension.
On Wayland, the system asks which screen to share when screen sync starts. X11 needs nothing.

### Windows

Nothing to set up.

## Use

Click the tray / menu-bar icon:

- **Open colour picker…**: changes apply live and are saved when you close the window
- **Light on**: turn the strip off and on
- **Screen sync**: mirror the screen edges; turning it off returns to your colour.
  The picker's brightness slider also dims screen sync.
- **Sync screen ›**: which screen to mirror (main screen by default). If the chosen screen is
  disconnected, the main screen is used.
- **Colours ›**: screen-sync colour preset (see below)
- **Resend colour**: reconnect and re-send, if the strip ever shows the rainbow again
- **Launch at login**: start automatically (enable it from the binary's final location)
- **Quit**: the strip keeps its last colour

### Colour presets

Averaging a patch of screen pulls colours towards grey, and screens store mid-tones brighter
than LEDs show them, so raw screen colours look washed out. The presets add saturation and a
gamma curve:

| Preset | Saturation | Gamma | |
|--------|-----------:|------:|---|
| Natural | 1.0 | 1.0 | colours as sampled |
| Vivid | 1.5 | 1.6 | default |
| Intense | 2.0 | 2.0 | strongest colours, darker mid-tones |

For fine-tuning, set `sync_saturation` and `sync_gamma` in `settings.json` directly.

### Settings file

Settings are stored in `settings.json`:

- macOS: `~/Library/Application Support/com.tanmaster.light-control/`
- Linux: `~/.config/light-control/`
- Windows: `%APPDATA%\tanmaster\light-control\config\`

Edit it while the app is not running.

### LED layout

The `layout` entry lists the strip's segments in LED order, each along one screen edge,
seen from the front. The default matches the 80-LED kit (LED 1 at the bottom of the left edge,
running clockwise):

```json
"layout": [
  { "edge": "left",   "count": 14, "reverse": false },
  { "edge": "top",    "count": 26, "reverse": false },
  { "edge": "right",  "count": 14, "reverse": false },
  { "edge": "bottom", "count": 26, "reverse": false }
]
```

`reverse: false` means clockwise: left bottom→top, top left→right, right top→bottom,
bottom right→left. Set `reverse: true` for a segment that runs the other way.

If your strip is mounted differently, quit the tray app and run

```sh
light-control --test-layout
```

LEDs 1–3 turn yellow, 4–14 red, 15–40 green, 41–54 blue and 55–80 white, so you can read off
where each segment sits. Replug the strip or start the app again afterwards.

## Build from source

Install Rust (1.80 or newer) from <https://rustup.rs>, then:

```sh
cargo test --release
cargo build --release
./target/release/light-control        # Windows: target\release\light-control.exe
```

- **macOS**: the Xcode Command Line Tools are enough (`xcode-select --install`); full Xcode is not needed.
- **Linux (Ubuntu / Debian)** build dependencies:

  ```sh
  sudo apt install build-essential pkg-config clang libgtk-3-dev libxdo-dev \
    libayatana-appindicator3-dev libudev-dev libxkbcommon-dev \
    libxcb1-dev libxrandr-dev libdbus-1-dev libpipewire-0.3-dev libwayland-dev libegl-dev libgbm-dev
  ```

While running from a terminal, the app prints connection errors and, during screen sync,
its frame rate and capture time every 5 seconds.

## How it works

The strip is a USB HID device. The packet formats (`RB` commands and `SC` screen-sync frames)
are documented in [`docs/protocol.md`](docs/protocol.md) and unit-tested byte for byte against
real USB captures. Notes on the controller board are in [`docs/hardware.md`](docs/hardware.md).

- `src/protocol.rs`: packet formats
- `src/device.rs`: background thread that owns the HID device, re-sends the colour when needed, streams sync frames
- `src/sync.rs`, `src/layout.rs`, `src/color.rs`: screen-sync loop, edge sampling, colour presets
- `src/capture_mac.rs`: ScreenCaptureKit capture on macOS; Windows and Linux use [`xcap`](https://crates.io/crates/xcap) screenshots
- `src/tray.rs`: tray icon and menu
- `src/picker.rs`: colour picker window, run as a separate short-lived process so the always-running part stays small
- `src/autostart.rs`: LaunchAgent (macOS), XDG autostart (Linux), `HKCU\…\Run` (Windows)

## Related projects

- [light-control](https://github.com/tanmaster/light-control): the Python predecessor of this app
- [OpenRGB](https://gitlab.com/CalcProgrammer1/OpenRGB): open-source RGB control for many devices, including a Robobloq controller
- [OpenLamp](https://github.com/danodoms/openlamp-robobloq-macos): a macOS app for Robobloq lights
- [AmbiEdge](https://github.com/KorbSoft/AmbiEdge): ambient screen lighting

## License

[GPL-3.0](LICENSE). Not affiliated with DX-Light or Robobloq. Only tested on the hardware
described above; sending commands to USB devices is at your own risk.
