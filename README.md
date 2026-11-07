# light-control (Rust)

A small, open-source replacement for the DX-Light app, for the generic USB monitor
backlight strip sold under many names (USB `1a86:fe07`, 80 LEDs).

It sits in the tray / menu bar and keeps the strip on **one static colour of your
choice**, instead of the built-in rainbow mode.

- colour picker (wheel, hex, brightness)
- light on/off
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
sudo apt install build-essential pkg-config libgtk-3-dev libxdo-dev \
  libayatana-appindicator3-dev libudev-dev libxkbcommon-dev
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
- **Resend colour**: force a reconnect if the strip ever shows the rainbow again
- **Launch at login**: start automatically (enable it from the final location of
  the binary, not from `target/debug`)
- **Quit**: the strip keeps its last colour

Settings live in `settings.json` in the OS config directory
(`~/Library/Application Support/com.tanmaster.light-control/` on macOS,
`~/.config/light-control/` on Linux, `%APPDATA%\tanmaster\light-control\config\` on Windows).
`led_count` can be changed there if your strip has a different number of LEDs.

## How it works

- `src/protocol.rs`: packet format (`"RB"` header, length, sequence, action, payload, checksum), unit-tested against real USB captures
- `src/device.rs`: background thread that owns the USB HID device and re-sends the colour whenever needed
- `src/tray.rs`: tray icon and menu
- `src/picker.rs`: the colour picker window, started as a separate short-lived process (`light-control --picker`) so the always-running part stays small
- `src/autostart.rs`: LaunchAgent (macOS), XDG autostart entry (Linux), `HKCU\…\Run` (Windows)

## Disclaimer

Only tested on the hardware described above. Sending raw commands to USB devices is at your own risk.
