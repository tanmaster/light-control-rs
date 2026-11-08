# light-control-rs

Open-source replacement for the closed-source DX-Light / Robobloq app that controls a
generic USB monitor backlight strip (USB `1a86:fe07`, 80 LEDs). Rust tray app for macOS,
Windows and Linux: static colour, brightness, on/off, screen sync (ambilight), launch at login.
Successor of the Python project https://github.com/tanmaster/light-control.

Primary dev machine: macOS (Apple Silicon), with only the Xcode **Command Line Tools**
(not full Xcode). Rust is installed via Homebrew, not rustup.

## Commands

```sh
cargo test --release          # protocol, layout, colour and picker tests (no hardware needed)
cargo build --release
./target/release/light-control                 # tray app (prints stats/errors to stderr)
./target/release/light-control --test-layout   # lights fixed LED ranges to check the layout
```

Linux check without a Linux machine (Docker / OrbStack; clippy, tests, build, runtime-library check):

```sh
tools/linux/check.sh
```

Run it before pushing Linux-specific changes instead of iterating on CI.

Quit any running instance before `--test-layout`, or it overwrites the pattern within 30 s.

## Architecture (src/)

- `main.rs`: dispatch: tray (default), `--picker` (colour picker process), `--test-layout`
- `protocol.rs`: wire format, unit-tested byte-for-byte against real USB captures. See `docs/protocol.md`
- `device.rs`: thread owning the HID device. Static mode re-sends the colour on replug,
  after wake (wall-clock jump), and every 30 s. Sync mode streams `SC` frames.
- `tray.rs`: tao event loop + tray-icon menu, owns `Settings`
- `picker.rs`: egui colour picker, runs as a **separate short-lived process** and reports
  `RRGGBB <brightness>` lines on stdout (keeps the tray process small, avoids event-loop conflicts)
- `sync.rs`: screen-sync loop (~30 fps, exponential smoothing, sends only on change)
- `capture_mac.rs`: macOS ScreenCaptureKit capture, GPU-scaled to 160×90, frames only on change
- `capture_linux.rs`: Wayland capture via xdg-desktop-portal ScreenCast (zbus, blocking) + PipeWire
  stream; reads a sparse grid of the full-res SHM frame into 160 px wide frames. Restore token
  (`persist_mode` 2) stored as `screencast-token` next to `settings.json`, keyed by screen choice
- `layout.rs`: LED positions around the screen + edge sampling
- `color.rs`: saturation + gamma enhancement presets (Natural / Vivid / Intense)
- `settings.rs`: JSON settings in the OS config dir (`directories` crate)
- `autostart.rs`: LaunchAgent / XDG autostart / HKCU Run, no extra crates

## Gotchas

- **macOS + Command Line Tools only:** `build.rs` adds the Swift runtime link path (via
  `xcrun --find swift`) and `-rpath /usr/lib/swift`. Without it, linking fails with
  `__swift_FORCE_LOAD_$_swiftCompatibility56` or the binary aborts with
  `Library not loaded: @rpath/libswift_Concurrency.dylib`. A dependency's
  `cargo:rustc-link-arg` does not reach our binary, so these must stay in our `build.rs`.
- **Linux runtime libraries:** the binary must only link libraries a stock Ubuntu desktop ships
  (CI checks this with `readelf`; libpipewire is allowed, Ubuntu desktop ships it). `tray-icon` is built without its default `libxdo` feature
  for this reason (v0.2.0 failed to start with `libxdo.so.3: cannot open shared object file`).
- **udev rule number:** `TAG+="uaccess"` only works in a rule file numbered below 73
  (systemd applies it in `73-seat-late.rules`), hence `70-light-control.rules`.
  v0.2.0/v0.2.1 shipped `99-…` and gave "Permission denied".
- Screen capture needs Screen Recording permission on macOS (granted to the terminal when
  run from one). Ubuntu Wayland shows a screen-share prompt (once, thanks to the restore token).
- **Never use xcap screenshots on GNOME Wayland:** they go through `org.gnome.Shell.Screenshot`,
  ~530 ms per frame, flash the screen white and block input. `sync.rs` uses PipeWire there and
  does not fall back to screenshots.
- The strip ignores the checksum in practice, but we always send correct ones.
- LED order (verified): LED 1 at the bottom of the left edge, clockwise: left 14, top 26,
  right 14, bottom 26. This is the default `layout` in settings.

## Status

Working and verified on macOS: static colour, picker, replug/wake recovery, autostart,
screen sync with monitor choice, ScreenCaptureKit capture (~1% CPU on a still screen),
colour presets. Release binary ~3.8 MB.

**Windows and Linux** build and pass tests in CI (`.github/workflows/build.yml`).
Windows: works on real hardware; screen sync uses `xcap` screenshots and is a bit laggy.
Linux: tray, picker and udev rule work on Ubuntu. Wayland PipeWire capture tested on
real hardware; X11 uses `xcap` screenshots. Linux needs `libpipewire-0.3-dev`, `libgbm-dev` and friends; the full apt list
is in the workflow and README.

## Releasing

Bump `version` in `Cargo.toml`, then push a tag `vX.Y.Z`. `.github/workflows/release.yml`
builds all three OSes and publishes a GitHub release with per-OS archives (binary, README,
LICENSE; Linux also the udev rule), the standalone udev rule and `SHA256SUMS.txt`.
The release workflow has not been run yet.

## Open tasks

1. Wayland screen sync (PipeWire) works on Ubuntu; still untested: switching screens,
   declining the dialog, stopping sharing from the top bar.
2. Native capture on Windows (screen sync is laggier there).

## Deferred / parked

- Native capture on Windows (Windows Graphics Capture) and X11 (edge strips via xcb).
- Custom firmware for the controller: parked by the owner. Findings in `docs/hardware.md`.
  The original firmware is read-protected and cannot be backed up, so flashing would be
  one-way. **Do not flash anything without explicit approval.**
