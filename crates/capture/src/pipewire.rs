//! Linux capture: xdg-desktop-portal ScreenCast -> PipeWire (spec 02).
//! Ported from fnf-thing-v2 `scanner/pipewire.rs`: portal handshake with a
//! restore token, PipeWire loop on its own thread, CPU crop of the region
//! into the frame ring, SHM only (no DMA-BUF in v0.1).

use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use arrowhead_core::frame::{CaptureError, FrameSource, FrameView, OwnedFrame, Rect, SourceInfo};
use arrowhead_core::runtime::platform::monotonic_ns;
use arrowhead_core::time::Nanos;
use pipewire as pw;
use pw::spa;
use spa::pod::Pod;
use spa::utils::{Direction, Fraction, Rectangle, SpaTypes};

use crate::ring::{Ring, RingStats};

/// pts further than this from now is not trusted.
const PTS_MAX_AGE_NS: u64 = 100_000_000;
/// Framerate asked for when the output refresh is unknown.
const FALLBACK_FPS: u32 = 240;

struct Shared {
    ring: Ring,
    width: AtomicU32,
    height: AtomicU32,
    /// Negotiated max framerate x 1000.
    fps_milli: AtomicU32,
    pts_is_receive_time: AtomicBool,
}

struct Quit;

/// Keeps the portal session (and the D-Bus connection under it) open for as long as we capture.
struct Portal {
    _rt: tokio::runtime::Runtime,
    _session: ashpd::desktop::Session<ashpd::desktop::screencast::Screencast>,
}

pub struct PipewireSource {
    shared: Arc<Shared>,
    monitor: String,
    quit_tx: Option<pw::channel::Sender<Quit>>,
    join: Option<JoinHandle<()>>,
    _portal: Portal,
}

fn backend(e: impl std::fmt::Display) -> CaptureError {
    CaptureError::Backend(e.to_string())
}

struct Handshake {
    fd: OwnedFd,
    node_id: u32,
    monitor: String,
    token: Option<String>,
    session: ashpd::desktop::Session<ashpd::desktop::screencast::Screencast>,
}

async fn handshake(restore_token: Option<&str>) -> Result<Handshake, CaptureError> {
    use ashpd::desktop::PersistMode;
    use ashpd::desktop::screencast::{CursorMode, Screencast, SelectSourcesOptions, SourceType};

    let proxy = Screencast::new()
        .await
        .map_err(|e| backend(format!("ScreenCast portal: {e}")))?;
    let session = proxy
        .create_session(Default::default())
        .await
        .map_err(|e| backend(format!("CreateSession: {e}")))?;
    let mut opts = SelectSourcesOptions::default()
        .set_cursor_mode(CursorMode::HIDDEN)
        .set_sources(SourceType::MONITOR)
        .set_multiple(false)
        .set_persist_mode(PersistMode::ExplicitlyRevoked);
    if let Some(t) = restore_token {
        opts = opts.set_restore_token(t);
    }
    proxy
        .select_sources(&session, opts)
        .await
        .map_err(|e| backend(format!("SelectSources: {e}")))?;
    let streams = proxy
        .start(&session, None, Default::default())
        .await
        .map_err(|e| backend(format!("Start: {e}")))?
        .response()
        .map_err(|e| match e {
            ashpd::Error::Response(ashpd::desktop::ResponseError::Cancelled) => {
                CaptureError::PermissionDenied
            }
            e => backend(format!("Start response: {e}")),
        })?;
    let stream = streams
        .streams()
        .first()
        .ok_or_else(|| backend("portal returned no streams"))?;
    let node_id = stream.pipe_wire_node_id();
    let monitor = stream.id().unwrap_or("portal").to_string();
    let token = streams.restore_token().map(str::to_string);
    let fd = proxy
        .open_pipe_wire_remote(&session, Default::default())
        .await
        .map_err(|e| backend(format!("OpenPipeWireRemote: {e}")))?;
    Ok(Handshake {
        fd,
        node_id,
        monitor,
        token,
        session,
    })
}

impl PipewireSource {
    /// Opens the portal picker (or restores the last choice from `token_file`)
    /// and waits for the first frame.
    pub fn open(token_file: &Path) -> Result<Self, CaptureError> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(backend)?;
        let stored = std::fs::read_to_string(token_file)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let hs = match rt.block_on(handshake(stored.as_deref())) {
            Ok(h) => h,
            Err(CaptureError::PermissionDenied) => return Err(CaptureError::PermissionDenied),
            Err(e) if stored.is_some() => {
                eprintln!("capture: restore token rejected ({e}); asking again");
                let _ = std::fs::remove_file(token_file);
                rt.block_on(handshake(None))?
            }
            Err(e) => return Err(e),
        };
        // The token is single-use: store the new one every run.
        if let Some(t) = &hs.token {
            if let Err(e) = save_token(token_file, t) {
                eprintln!("capture: could not save restore token: {e}");
            }
        }

        let shared = Arc::new(Shared {
            ring: Ring::new(),
            width: AtomicU32::new(0),
            height: AtomicU32::new(0),
            fps_milli: AtomicU32::new(0),
            pts_is_receive_time: AtomicBool::new(false),
        });
        let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<(), String>>(1);
        let (quit_tx, quit_rx) = pw::channel::channel::<Quit>();
        let s2 = shared.clone();
        let (fd, node_id) = (hs.fd, hs.node_id);
        let join = std::thread::Builder::new()
            .name("ah-pipewire".into())
            .spawn(move || {
                if let Err(e) = run_loop(fd, node_id, s2.clone(), ready_tx.clone(), quit_rx) {
                    let _ = ready_tx.send(Err(e.clone()));
                    s2.ring.close(e, true);
                }
            })
            .map_err(backend)?;

        let src = Self {
            shared,
            monitor: hs.monitor,
            quit_tx: Some(quit_tx),
            join: Some(join),
            _portal: Portal {
                _rt: rt,
                _session: hs.session,
            },
        };
        match ready_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(())) => Ok(src),
            Ok(Err(e)) => Err(backend(e)),
            Err(_) => Err(backend("no frame from PipeWire within 5 s")),
        }
    }

    pub fn stats(&self) -> RingStats {
        self.shared.ring.stats()
    }
}

fn save_token(path: &Path, token: &str) -> std::io::Result<()> {
    arrowhead_core::config::write_atomic(path, token.as_bytes())
}

/// Default token location: `<data_dir>/screencast_token`.
pub fn default_token_file() -> Option<PathBuf> {
    arrowhead_core::config::data_dir().map(|d| d.join("screencast_token"))
}

struct UserData {
    shared: Arc<Shared>,
    format: Option<(u32, u32)>,
    ready: Option<mpsc::SyncSender<Result<(), String>>>,
    warned_dmabuf: bool,
    warned_pts: bool,
}

fn run_loop(
    fd: OwnedFd,
    node_id: u32,
    shared: Arc<Shared>,
    ready: mpsc::SyncSender<Result<(), String>>,
    quit_rx: pw::channel::Receiver<Quit>,
) -> Result<(), String> {
    pw::init();
    fn e(what: &'static str) -> impl Fn(pw::Error) -> String {
        move |err| format!("{what}: {err}")
    }
    let mainloop = pw::main_loop::MainLoopRc::new(None).map_err(e("main loop"))?;
    let context = pw::context::ContextRc::new(&mainloop, None).map_err(e("context"))?;
    let core = context
        .connect_fd_rc(fd, None)
        .map_err(e("connect portal remote"))?;
    let _quit = quit_rx.attach(mainloop.loop_(), {
        let ml = mainloop.clone();
        move |Quit| ml.quit()
    });

    let props = pw::properties::properties! {
        *pw::keys::MEDIA_TYPE => "Video",
        *pw::keys::MEDIA_CATEGORY => "Capture",
        *pw::keys::MEDIA_ROLE => "Screen",
    };
    let stream =
        pw::stream::StreamBox::new(&core, "arrowhead capture", props).map_err(e("stream"))?;
    let data = UserData {
        shared: shared.clone(),
        format: None,
        ready: Some(ready),
        warned_dmabuf: false,
        warned_pts: false,
    };
    let lost = shared.clone();

    let _listener = stream
        .add_local_listener_with_user_data(data)
        .state_changed(move |_, _, _, new| {
            if let pw::stream::StreamState::Error(msg) = new {
                lost.ring.close(msg, true);
            } else if matches!(new, pw::stream::StreamState::Unconnected) {
                lost.ring.close("stream disconnected".into(), true);
            }
        })
        .param_changed(|stream, data, id, param| {
            let Some(param) = param else { return };
            if id != spa::param::ParamType::Format.as_raw() {
                return;
            }
            let Ok((mt, mst)) = spa::param::format_utils::parse_format(param) else {
                return;
            };
            if mt != spa::param::format::MediaType::Video
                || mst != spa::param::format::MediaSubtype::Raw
            {
                return;
            }
            let mut info = spa::param::video::VideoInfoRaw::default();
            if info.parse(param).is_err() {
                return;
            }
            let size = info.size();
            data.format = Some((size.width, size.height));
            data.shared.width.store(size.width, Ordering::Relaxed);
            data.shared.height.store(size.height, Ordering::Relaxed);
            let fr = if info.max_framerate().num > 0 {
                info.max_framerate()
            } else {
                info.framerate()
            };
            if let Some(fps) = (fr.num * 1000).checked_div(fr.denom) {
                data.shared.fps_milli.store(fps, Ordering::Relaxed);
            }
            // Ask for the header meta so frames carry their presentation time.
            if let Ok(meta) = header_meta_param() {
                let _ = stream.update_params(&mut [meta]);
            }
        })
        .process(|stream, data| {
            let Some(mut buffer) = stream.dequeue_buffer() else {
                return;
            };
            let now = monotonic_ns();
            let pts = buffer
                .find_meta::<spa::buffer::meta::MetaHeader>()
                .map(|h| h.pts())
                .filter(|&p| p > 0 && (now as i64 - p).unsigned_abs() < PTS_MAX_AGE_NS)
                .map(|p| p as u64);
            if pts.is_none() && !data.warned_pts {
                eprintln!(
                    "capture: frame pts missing or not on CLOCK_MONOTONIC; using receive time"
                );
                data.warned_pts = true;
                data.shared
                    .pts_is_receive_time
                    .store(true, Ordering::Relaxed);
            }
            let Some((w, h)) = data.format else { return };
            let Some(d) = buffer.datas_mut().first_mut() else {
                return;
            };
            if d.type_() == spa::buffer::DataType::DmaBuf {
                if !data.warned_dmabuf {
                    eprintln!("capture: got a DMA-BUF buffer despite asking for SHM; skipping");
                    data.warned_dmabuf = true;
                }
                return;
            }
            let stride = match d.chunk().stride() {
                s if s > 0 => s as usize,
                _ => w as usize * 4,
            };
            let offset = d.chunk().offset() as usize;
            let size = d.chunk().size() as usize;
            let Some(bytes) = d.data() else { return };
            let Some(bytes) = bytes.get(offset..offset + size) else {
                return;
            };

            let full = Rect::new(0, 0, w, h);
            data.shared.ring.produce(|slot, want| {
                let r = if want.w == 0 || want.h == 0 {
                    full
                } else {
                    want.intersect(&full).unwrap_or(full)
                };
                let row = r.w as usize * 4;
                let need = row * r.h as usize;
                if slot.data.len() < need {
                    slot.data.resize(need, 0);
                }
                for y in 0..r.h as usize {
                    let src = (r.y as usize + y) * stride + r.x as usize * 4;
                    let Some(line) = bytes.get(src..src + row) else {
                        return false;
                    };
                    slot.data[y * row..(y + 1) * row].copy_from_slice(line);
                }
                slot.region = r;
                slot.stride = row;
                slot.pts = Nanos(pts.unwrap_or(now));
                true
            });
            if let Some(tx) = data.ready.take() {
                let _ = tx.send(Ok(()));
            }
        })
        .register()
        .map_err(e("register listener"))?;

    let fmt = enum_format_param()?;
    stream
        .connect(
            Direction::Input,
            Some(node_id),
            pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
            &mut [fmt],
        )
        .map_err(e("connect stream"))?;
    mainloop.run();
    shared.ring.close("capture stopped".into(), false);
    Ok(())
}

fn to_pod(obj: spa::pod::Object) -> Result<&'static Pod, String> {
    let bytes = spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &spa::pod::Value::Object(obj),
    )
    .map_err(|e| format!("serialize pod: {e:?}"))?
    .0
    .into_inner();
    // Leaked on purpose: built once per stream, and connect/update_params need it to outlive the call.
    Pod::from_bytes(Box::leak(bytes.into_boxed_slice())).ok_or_else(|| "bad pod".to_string())
}

/// Raw video, BGRx or BGRA (both read as BGRx), any size, up to 1000 fps.
fn enum_format_param() -> Result<&'static Pod, String> {
    use spa::param::format::{FormatProperties, MediaSubtype, MediaType};
    use spa::param::video::VideoFormat;
    to_pod(spa::pod::object!(
        SpaTypes::ObjectParamFormat,
        spa::param::ParamType::EnumFormat,
        spa::pod::property!(FormatProperties::MediaType, Id, MediaType::Video),
        spa::pod::property!(FormatProperties::MediaSubtype, Id, MediaSubtype::Raw),
        spa::pod::property!(
            FormatProperties::VideoFormat,
            Choice,
            Enum,
            Id,
            VideoFormat::BGRx,
            VideoFormat::BGRx,
            VideoFormat::BGRA
        ),
        spa::pod::property!(
            FormatProperties::VideoSize,
            Choice,
            Range,
            Rectangle,
            Rectangle {
                width: 2560,
                height: 1440
            },
            Rectangle {
                width: 1,
                height: 1
            },
            Rectangle {
                width: 16384,
                height: 16384
            }
        ),
        spa::pod::property!(
            FormatProperties::VideoFramerate,
            Choice,
            Range,
            Fraction,
            Fraction {
                num: FALLBACK_FPS,
                denom: 1
            },
            Fraction { num: 0, denom: 1 },
            Fraction {
                num: 1000,
                denom: 1
            }
        ),
    ))
}

fn header_meta_param() -> Result<&'static Pod, String> {
    use spa::sys;
    to_pod(spa::pod::Object {
        type_: SpaTypes::ObjectParamMeta.as_raw(),
        id: spa::param::ParamType::Meta.as_raw(),
        properties: vec![
            spa::pod::Property::new(
                sys::SPA_PARAM_META_type,
                spa::pod::Value::Id(spa::utils::Id(sys::SPA_META_Header)),
            ),
            spa::pod::Property::new(
                sys::SPA_PARAM_META_size,
                spa::pod::Value::Int(std::mem::size_of::<sys::spa_meta_header>() as i32),
            ),
        ],
    })
}

impl FrameSource for PipewireSource {
    fn next_frame(&mut self, timeout: Duration) -> Result<Option<FrameView<'_>>, CaptureError> {
        Ok(self.shared.ring.acquire(timeout)?.map(|s| s.view()))
    }

    fn set_region(&mut self, region: Rect) {
        self.shared.ring.set_region(region);
    }

    fn info(&self) -> SourceInfo {
        let s = &self.shared;
        let fps = s.fps_milli.load(Ordering::Relaxed);
        SourceInfo {
            width: s.width.load(Ordering::Relaxed),
            height: s.height.load(Ordering::Relaxed),
            refresh_hz: fps as f32 / 1000.0,
            backend: "pipewire".into(),
            monitor: self.monitor.clone(),
            scale: 1.0,
            pts_is_receive_time: s.pts_is_receive_time.load(Ordering::Relaxed),
        }
    }

    fn snapshot(&mut self) -> Result<OwnedFrame, CaptureError> {
        let ring = &self.shared.ring;
        let info = self.info();
        let prev = ring.region();
        ring.set_region(Rect::default());
        let deadline = Instant::now() + Duration::from_secs(1);
        let out = loop {
            match ring.acquire(deadline.saturating_duration_since(Instant::now())) {
                Ok(Some(s)) if s.region.w == info.width && s.region.h == info.height => {
                    break Ok(s.view().to_owned());
                }
                Ok(Some(_)) => continue,
                Ok(None) => break Err(backend("no full frame within 1 s")),
                Err(e) => break Err(e),
            }
        };
        ring.set_region(prev);
        out
    }
}

impl Drop for PipewireSource {
    fn drop(&mut self) {
        if let Some(tx) = self.quit_tx.take() {
            let _ = tx.send(Quit);
        }
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}
