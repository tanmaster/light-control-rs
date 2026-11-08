//! Linux (Wayland) screen capture via the xdg-desktop-portal ScreenCast API and PipeWire.
//!
//! On GNOME Wayland, `xcap` screenshots go through `org.gnome.Shell.Screenshot`: one PNG per
//! frame, ~500 ms each, with a white flash, and the shell blocks input while it runs.
//! A ScreenCast session instead streams frames, and only when the screen changes.
//!
//! The portal asks once which screen to share. With `persist_mode = 2` it hands out a restore
//! token, which we keep next to the settings so later sessions start without a dialog.
//! Frames arrive at full resolution in shared memory; we only read a sparse grid of pixels
//! and hand a small (160 px wide) frame to the sampler, like the macOS capture does.

use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use pipewire as pw;
use pw::spa::param::format::{FormatProperties, MediaSubtype, MediaType};
use pw::spa::param::video::{VideoFormat, VideoInfoRaw};
use pw::spa::param::{format_utils, ParamType};
use pw::spa::pod::{self, serialize::PodSerializer, Pod};
use pw::spa::utils::{Direction, Fraction, Rectangle, SpaTypes};
use pw::stream::{StreamFlags, StreamRc, StreamState};
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{self, DeserializeDict, OwnedObjectPath, OwnedValue, Type, Value};

use crate::sync::Frame;

/// Width of the frames handed to the sampler; the height follows the screen's aspect ratio.
const OUTPUT_WIDTH: u32 = 160;
/// Pixels read per output pixel and axis (4 → 16 samples per output pixel).
const SAMPLES: u32 = 4;
const FPS: u32 = 30;
/// File next to `settings.json` holding the screen choice and the portal's restore token.
const TOKEN_FILE: &str = "screencast-token";

/// Whether we run in a Wayland session (where screenshots are slow and flash on GNOME).
pub fn is_wayland() -> bool {
    std::env::var("XDG_SESSION_TYPE").is_ok_and(|v| v.eq_ignore_ascii_case("wayland"))
        || std::env::var_os("WAYLAND_DISPLAY").is_some()
}

/// Whether the desktop is GNOME, where xcap's screenshot fallback flashes the screen.
pub fn is_gnome() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|v| v.to_ascii_uppercase().contains("GNOME"))
}

#[derive(Debug)]
pub enum Error {
    /// The user closed the screen-share dialog: don't ask again until sync is restarted.
    Cancelled,
    Other(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => f.write_str("screen sharing was cancelled"),
            Self::Other(e) => f.write_str(e),
        }
    }
}

impl From<zbus::Error> for Error {
    fn from(e: zbus::Error) -> Self {
        Self::Other(format!("portal: {e}"))
    }
}

impl From<zvariant::Error> for Error {
    fn from(e: zvariant::Error) -> Self {
        Self::Other(format!("portal: {e}"))
    }
}

impl From<pw::Error> for Error {
    fn from(e: pw::Error) -> Self {
        Self::Other(format!("pipewire: {e}"))
    }
}

#[derive(Default)]
struct Shared {
    latest: Mutex<Option<Frame>>,
    frames: AtomicU64,
    /// Set when the stream ends or fails (e.g. sharing stopped from the top bar).
    failed: Mutex<Option<String>>,
}

pub struct LinuxCapture {
    shared: Arc<Shared>,
    seen: u64,
    quit: pw::channel::Sender<()>,
    thread: Option<JoinHandle<()>>,
    portal: Portal,
    session: OwnedObjectPath,
}

impl LinuxCapture {
    /// Open a ScreenCast session (asking the user which screen to share unless a stored
    /// restore token for `screen` is still valid) and start streaming it.
    pub fn start(screen: Option<&str>) -> Result<Self, Error> {
        let portal = Portal::new()?;
        let screen = screen.unwrap_or("");
        let restore = load_token(screen);

        let session = portal.create_session()?;
        let started = portal
            .select_sources(&session, restore.as_deref())
            .and_then(|()| portal.start(&session));
        let started = match started {
            Ok(s) => s,
            Err(e) => {
                portal.close(&session);
                return Err(e);
            }
        };
        if let Some(token) = &started.restore_token {
            save_token(screen, token);
        }
        let Some(node) = started.streams.as_ref().and_then(|s| s.first()).map(|s| s.0) else {
            portal.close(&session);
            return Err(Error::Other("the portal returned no stream".into()));
        };
        let fd = match portal.open_pipewire_remote(&session) {
            Ok(fd) => fd,
            Err(e) => {
                portal.close(&session);
                return Err(e);
            }
        };

        let shared = Arc::new(Shared::default());
        let (quit, quit_rx) = pw::channel::channel::<()>();
        let (ready_tx, ready_rx) = mpsc::channel();
        let thread = {
            let shared = shared.clone();
            thread::Builder::new()
                .name("pipewire-capture".into())
                .spawn(move || {
                    if let Err(e) = run_stream(fd, node, &shared, quit_rx, &ready_tx) {
                        let _ = ready_tx.send(Err(e.to_string()));
                    }
                })
                .map_err(|e| Error::Other(format!("cannot start capture thread: {e}")))?
        };

        let mut capture = Self { shared, seen: 0, quit, thread: Some(thread), portal, session };
        match ready_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(())) => Ok(capture),
            Ok(Err(e)) => {
                capture.shutdown();
                Err(Error::Other(e))
            }
            Err(_) => {
                capture.shutdown();
                Err(Error::Other("pipewire stream did not start".into()))
            }
        }
    }

    /// The newest frame, if one arrived since the last call.
    pub fn next_frame(&mut self) -> Result<Option<Frame>, String> {
        if let Some(e) = self.shared.failed.lock().ok().and_then(|f| f.clone()) {
            return Err(e);
        }
        let count = self.shared.frames.load(Ordering::Acquire);
        if count == self.seen {
            return Ok(None);
        }
        self.seen = count;
        Ok(self.shared.latest.lock().ok().and_then(|mut l| l.take()))
    }

    fn shutdown(&mut self) {
        let _ = self.quit.send(());
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        self.portal.close(&self.session);
    }
}

impl Drop for LinuxCapture {
    fn drop(&mut self) {
        self.shutdown();
    }
}

// ---------------------------------------------------------------------------------------------
// Portal (D-Bus)
// ---------------------------------------------------------------------------------------------

const PORTAL_DEST: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";

#[derive(DeserializeDict, Type, Debug)]
#[zvariant(signature = "dict")]
struct StartResults {
    streams: Option<Vec<(u32, HashMap<String, OwnedValue>)>>,
    restore_token: Option<String>,
}

struct Portal {
    conn: Connection,
    proxy: Proxy<'static>,
    /// Our unique bus name in the form the portal uses in object paths.
    sender: String,
}

impl Portal {
    fn new() -> Result<Self, Error> {
        let conn = Connection::session()?;
        let proxy =
            Proxy::new(&conn, PORTAL_DEST, PORTAL_PATH, "org.freedesktop.portal.ScreenCast")?;
        let sender = conn
            .unique_name()
            .ok_or_else(|| Error::Other("no D-Bus unique name".into()))?
            .trim_start_matches(':')
            .replace('.', "_");
        Ok(Self { conn, proxy, sender })
    }

    /// Call a portal method that answers through the `Response` signal of a Request object.
    fn request<B, R>(&self, method: &str, token: &str, body: &B) -> Result<R, Error>
    where
        B: serde::Serialize + zvariant::DynamicType,
        R: for<'de> serde::Deserialize<'de> + Type,
    {
        let path = format!("{PORTAL_PATH}/request/{}/{token}", self.sender);
        let request = Proxy::new(&self.conn, PORTAL_DEST, path, "org.freedesktop.portal.Request")?;
        // Subscribe before calling, or a fast answer could be missed.
        let mut responses = request.receive_signal("Response")?;
        self.proxy.call_method(method, body)?;
        let message = responses
            .next()
            .ok_or_else(|| Error::Other(format!("portal: no answer to {method}")))?;
        let (code, results): (u32, R) = message.body().deserialize()?;
        match code {
            0 => Ok(results),
            1 => Err(Error::Cancelled),
            c => Err(Error::Other(format!("portal: {method} failed (code {c})"))),
        }
    }

    fn create_session(&self) -> Result<OwnedObjectPath, Error> {
        let token = new_token();
        let session_token = new_token();
        let options = HashMap::from([
            ("handle_token", Value::from(token.as_str())),
            ("session_handle_token", Value::from(session_token.as_str())),
        ]);
        let _: HashMap<String, OwnedValue> = self.request("CreateSession", &token, &(options,))?;
        let path = format!("{PORTAL_PATH}/session/{}/{session_token}", self.sender);
        Ok(OwnedObjectPath::try_from(path)?)
    }

    fn select_sources(&self, session: &OwnedObjectPath, restore: Option<&str>) -> Result<(), Error> {
        let token = new_token();
        let mut options = HashMap::from([
            ("handle_token", Value::from(token.as_str())),
            ("types", Value::from(1u32)), // monitors
            ("multiple", Value::from(false)),
            ("persist_mode", Value::from(2u32)), // until the user revokes it
        ]);
        if let Some(restore) = restore {
            options.insert("restore_token", Value::from(restore));
        }
        let _: HashMap<String, OwnedValue> =
            self.request("SelectSources", &token, &(session, options))?;
        Ok(())
    }

    fn start(&self, session: &OwnedObjectPath) -> Result<StartResults, Error> {
        let token = new_token();
        let options = HashMap::from([("handle_token", Value::from(token.as_str()))]);
        self.request("Start", &token, &(session, "", options))
    }

    fn open_pipewire_remote(&self, session: &OwnedObjectPath) -> Result<std::os::fd::OwnedFd, Error> {
        let options: HashMap<&str, Value<'_>> = HashMap::new();
        let fd: zvariant::OwnedFd = self.proxy.call("OpenPipeWireRemote", &(session, options))?;
        Ok(fd.into())
    }

    /// End the session, which also removes the "sharing your screen" indicator.
    fn close(&self, session: &OwnedObjectPath) {
        if let Ok(proxy) =
            Proxy::new(&self.conn, PORTAL_DEST, session.as_str(), "org.freedesktop.portal.Session")
        {
            let _ = proxy.call_method("Close", &());
        }
    }
}

/// A token for portal object paths (letters, digits and `_` only), unique within the process.
fn new_token() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    format!("light_control_{}_{}", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed))
}

/// The stored restore token, if it was issued for the same screen choice.
fn load_token(screen: &str) -> Option<String> {
    let text = std::fs::read_to_string(crate::settings::config_file(TOKEN_FILE)?).ok()?;
    let (saved_screen, token) = text.split_once('\n')?;
    let token = token.trim();
    (saved_screen == screen && !token.is_empty()).then(|| token.to_string())
}

fn save_token(screen: &str, token: &str) {
    let Some(path) = crate::settings::config_file(TOKEN_FILE) else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(&path, format!("{screen}\n{token}\n")) {
        eprintln!("screen sync: could not save {}: {e}", path.display());
    }
}

// ---------------------------------------------------------------------------------------------
// PipeWire stream
// ---------------------------------------------------------------------------------------------

/// Runs on the capture thread until `quit` fires. Reports on `ready` once the stream is set up.
fn run_stream(
    fd: std::os::fd::OwnedFd,
    node: u32,
    shared: &Arc<Shared>,
    quit: pw::channel::Receiver<()>,
    ready: &mpsc::Sender<Result<(), String>>,
) -> Result<(), Error> {
    pw::init();
    let main_loop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&main_loop, None)?;
    let core = context.connect_fd_rc(fd, None)?;
    let stream = StreamRc::new(
        core,
        "light-control",
        pw::properties::properties! {
            *pw::keys::MEDIA_TYPE => "Video",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "Screen",
        },
    )?;

    let on_state = shared.clone();
    let on_frame = shared.clone();
    let _listener = stream
        .add_local_listener_with_user_data(VideoInfoRaw::default())
        .state_changed(move |_, _, _, new| {
            let reason = match new {
                StreamState::Error(e) => format!("stream failed: {e}"),
                StreamState::Unconnected => "screen sharing stopped".to_string(),
                _ => return,
            };
            if let Ok(mut failed) = on_state.failed.lock() {
                failed.get_or_insert(reason);
            }
        })
        .param_changed(|_, info, id, param| {
            let Some(param) = param else { return };
            if id != ParamType::Format.as_raw() {
                return;
            }
            let Ok((MediaType::Video, MediaSubtype::Raw)) = format_utils::parse_format(param) else {
                return;
            };
            if let Err(e) = info.parse(param) {
                eprintln!("screen sync: cannot parse video format: {e:?}");
            }
        })
        .process(move |stream, info| {
            let Some(mut buffer) = stream.dequeue_buffer() else { return };
            let Some(order) = ChannelOrder::of(info.format()) else { return };
            let size = info.size();
            let Some(data) = buffer.datas_mut().first_mut() else { return };
            let (offset, chunk_size, stride) =
                (data.chunk().offset() as usize, data.chunk().size() as usize, data.chunk().stride());
            if chunk_size == 0 {
                return; // no new picture in this buffer (e.g. only the cursor moved)
            }
            let Some(bytes) = data.data() else { return };
            let stride = if stride > 0 { stride as usize } else { size.width as usize * 4 };
            let Some(pixels) = bytes.get(offset..) else { return };
            if let Some(frame) = downscale(pixels, size.width, size.height, stride, order) {
                if let Ok(mut latest) = on_frame.latest.lock() {
                    *latest = Some(frame);
                }
                on_frame.frames.fetch_add(1, Ordering::Release);
            }
        })
        .register()?;

    let format = pod::object!(
        SpaTypes::ObjectParamFormat,
        ParamType::EnumFormat,
        pod::property!(FormatProperties::MediaType, Id, MediaType::Video),
        pod::property!(FormatProperties::MediaSubtype, Id, MediaSubtype::Raw),
        pod::property!(
            FormatProperties::VideoFormat,
            Choice,
            Enum,
            Id,
            VideoFormat::BGRx,
            VideoFormat::BGRx,
            VideoFormat::BGRA,
            VideoFormat::RGBx,
            VideoFormat::RGBA,
            VideoFormat::xRGB,
            VideoFormat::ARGB,
            VideoFormat::xBGR,
            VideoFormat::ABGR
        ),
        pod::property!(
            FormatProperties::VideoSize,
            Choice,
            Range,
            Rectangle,
            Rectangle { width: 1920, height: 1080 },
            Rectangle { width: 1, height: 1 },
            Rectangle { width: 8192, height: 8192 }
        ),
        pod::property!(
            FormatProperties::VideoFramerate,
            Choice,
            Range,
            Fraction,
            Fraction { num: FPS, denom: 1 },
            Fraction { num: 0, denom: 1 },
            Fraction { num: FPS, denom: 1 }
        ),
    );
    let bytes = PodSerializer::serialize(std::io::Cursor::new(Vec::new()), &pod::Value::Object(format))
        .map_err(|e| Error::Other(format!("pipewire: cannot build format: {e:?}")))?
        .0
        .into_inner();
    let mut params = [Pod::from_bytes(&bytes).ok_or_else(|| Error::Other("pipewire: bad format pod".into()))?];

    stream.connect(
        Direction::Input,
        Some(node),
        StreamFlags::AUTOCONNECT | StreamFlags::MAP_BUFFERS,
        &mut params,
    )?;

    let _quit = quit.attach(main_loop.loop_(), {
        let main_loop = main_loop.clone();
        move |()| main_loop.quit()
    });

    let _ = ready.send(Ok(()));
    main_loop.run();
    let _ = stream.disconnect();
    Ok(())
}

/// Byte positions of red, green and blue within a 4-byte pixel.
#[derive(Debug, Clone, Copy, PartialEq)]
struct ChannelOrder([usize; 3]);

impl ChannelOrder {
    fn of(format: VideoFormat) -> Option<Self> {
        Some(Self(match format {
            VideoFormat::RGBx | VideoFormat::RGBA => [0, 1, 2],
            VideoFormat::BGRx | VideoFormat::BGRA => [2, 1, 0],
            VideoFormat::xRGB | VideoFormat::ARGB => [1, 2, 3],
            VideoFormat::xBGR | VideoFormat::ABGR => [3, 2, 1],
            _ => return None,
        }))
    }
}

/// Shrink a 4-byte-per-pixel image to `OUTPUT_WIDTH` columns, averaging a sparse
/// `SAMPLES × SAMPLES` grid per output pixel. Returns `None` if the buffer is too short.
fn downscale(data: &[u8], width: u32, height: u32, stride: usize, order: ChannelOrder) -> Option<Frame> {
    if width == 0 || height == 0 {
        return None;
    }
    let out_w = OUTPUT_WIDTH.min(width);
    let out_h = ((u64::from(height) * u64::from(out_w) / u64::from(width)) as u32).max(1);
    let [r, g, b] = order.0;
    let mut rgba = Vec::with_capacity((out_w * out_h * 4) as usize);
    for oy in 0..out_h {
        for ox in 0..out_w {
            let mut sum = [0u32; 3];
            for sy in 0..SAMPLES {
                // Sample points at the centres of an even SAMPLES×SAMPLES grid inside the block.
                let y = (u64::from(oy) * 2 * u64::from(SAMPLES) + 2 * u64::from(sy) + 1)
                    * u64::from(height)
                    / (2 * u64::from(SAMPLES) * u64::from(out_h));
                let row = y as usize * stride;
                for sx in 0..SAMPLES {
                    let x = (u64::from(ox) * 2 * u64::from(SAMPLES) + 2 * u64::from(sx) + 1)
                        * u64::from(width)
                        / (2 * u64::from(SAMPLES) * u64::from(out_w));
                    let px = data.get(row + x as usize * 4..row + x as usize * 4 + 4)?;
                    sum[0] += u32::from(px[r]);
                    sum[1] += u32::from(px[g]);
                    sum[2] += u32::from(px[b]);
                }
            }
            let n = SAMPLES * SAMPLES;
            rgba.extend_from_slice(&[(sum[0] / n) as u8, (sum[1] / n) as u8, (sum[2] / n) as u8, 255]);
        }
    }
    Some(Frame { rgba, width: out_w, height: out_h })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A BGRx image whose left half is red and right half is blue, with padded rows.
    fn halves(width: u32, height: u32, stride: usize) -> Vec<u8> {
        let mut data = vec![0u8; stride * height as usize];
        for y in 0..height as usize {
            for x in 0..width as usize {
                let px = &mut data[y * stride + x * 4..][..4];
                px.copy_from_slice(if x < width as usize / 2 { &[0, 0, 255, 0] } else { &[255, 0, 0, 0] });
            }
        }
        data
    }

    #[test]
    fn downscales_to_output_width_keeping_aspect() {
        let (w, h, stride) = (1920, 1080, 1920 * 4 + 64);
        let frame = downscale(&halves(w, h, stride), w, h, stride, ChannelOrder([2, 1, 0])).unwrap();
        assert_eq!((frame.width, frame.height), (160, 90));
        assert_eq!(frame.rgba.len(), 160 * 90 * 4);
        assert_eq!(&frame.rgba[..4], &[255, 0, 0, 255]); // top left: red
        let last = frame.rgba.len() - 4;
        assert_eq!(&frame.rgba[last..], &[0, 0, 255, 255]); // bottom right: blue
    }

    #[test]
    fn small_images_are_not_upscaled() {
        let (w, h) = (64, 32);
        let frame = downscale(&halves(w, h, 256), w, h, 256, ChannelOrder([2, 1, 0])).unwrap();
        assert_eq!((frame.width, frame.height), (64, 32));
    }

    #[test]
    fn short_buffers_are_rejected() {
        assert!(downscale(&[0; 100], 1920, 1080, 1920 * 4, ChannelOrder([0, 1, 2])).is_none());
    }

    #[test]
    fn channel_orders() {
        assert_eq!(ChannelOrder::of(VideoFormat::BGRx), Some(ChannelOrder([2, 1, 0])));
        assert_eq!(ChannelOrder::of(VideoFormat::xRGB), Some(ChannelOrder([1, 2, 3])));
        assert_eq!(ChannelOrder::of(VideoFormat::NV12), None);
    }
}
