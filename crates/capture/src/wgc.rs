//! Windows Graphics Capture fallback (spec 02), through the `windows-capture` crate:
//! primary monitor, or the game's window (opt-in, coordinates are window-relative).
//! The crate delivers frames on its own thread; the handler crops the region into the ring.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};

use arrowhead_core::frame::{
    AuxTap, CaptureError, FrameSource, FrameView, OwnedFrame, Rect, SourceInfo,
};
use arrowhead_core::input::exe_matches;
use arrowhead_core::runtime::platform::monotonic_ns;
use arrowhead_core::time::Nanos;
use windows_capture::capture::{CaptureControl, Context, GraphicsCaptureApiHandler};
use windows_capture::frame::Frame;
use windows_capture::graphics_capture_api::InternalCaptureControl;
use windows_capture::monitor::Monitor;
use windows_capture::settings::{
    ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
    MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
};
use windows_capture::window::Window;

use crate::ring::{Ring, RingStats};
use crate::winpolicy::Kind;

struct Shared {
    ring: Ring,
    width: AtomicU32,
    height: AtomicU32,
    pts_is_receive_time: AtomicBool,
    aux: Arc<AuxTap>,
}

struct Handler {
    shared: Arc<Shared>,
    n: u64,
}

impl GraphicsCaptureApiHandler for Handler {
    type Flags = Arc<Shared>;
    type Error = String;

    fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
        arrowhead_core::runtime::platform::make_realtime();
        Ok(Self {
            shared: ctx.flags,
            n: 0,
        })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        control: InternalCaptureControl,
    ) -> Result<(), Self::Error> {
        let s = &self.shared;
        let (fw, fh) = (frame.width(), frame.height());
        s.width.store(fw, Ordering::Relaxed);
        s.height.store(fh, Ordering::Relaxed);
        // SystemRelativeTime is in 100 ns units. Assumed to share QPC's clock (open question).
        let pts = match frame.timestamp() {
            Ok(t) => Nanos(t.Duration.max(0) as u64 * 100),
            Err(_) => {
                s.pts_is_receive_time.store(true, Ordering::Relaxed);
                Nanos(monotonic_ns())
            }
        };
        self.n += 1;
        if s.aux.due(self.n).is_some() {
            // Full-frame map, only on the frames the HUD asked for.
            match frame.buffer() {
                Ok(mut buf) => {
                    let stride = buf.row_pitch() as usize;
                    let raw = buf.as_raw_buffer();
                    s.aux.copy_from(
                        self.n,
                        &FrameView {
                            seq: self.n,
                            pts,
                            region: Rect::new(0, 0, fw, fh),
                            stride,
                            data: raw,
                        },
                    );
                }
                Err(e) => eprintln!("capture: aux copy: {e}"),
            }
        }
        let mut err = None;
        s.ring.produce(|slot, want| {
            let want = if want.w == 0 || want.h == 0 {
                Rect::new(0, 0, fw, fh)
            } else {
                want
            };
            let Some(r) = want.intersect(&Rect::new(0, 0, fw, fh)) else {
                return false;
            };
            let (x0, y0) = (r.x as u32, r.y as u32);
            match frame.buffer_crop(x0, y0, x0 + r.w, y0 + r.h) {
                Ok(mut buf) => {
                    let pitch = buf.row_pitch() as usize;
                    let row = r.w as usize * 4;
                    let raw = buf.as_raw_buffer();
                    slot.data.resize(row * r.h as usize, 0);
                    for y in 0..r.h as usize {
                        slot.data[y * row..(y + 1) * row]
                            .copy_from_slice(&raw[y * pitch..y * pitch + row]);
                    }
                    slot.region = r;
                    slot.stride = row;
                    slot.pts = pts;
                    true
                }
                Err(e) => {
                    err = Some(e.to_string());
                    false
                }
            }
        });
        if let Some(e) = err {
            s.ring.close(format!("WGC frame: {e}"), false);
            control.stop();
        }
        Ok(())
    }

    fn on_closed(&mut self) -> Result<(), Self::Error> {
        self.shared.ring.close("capture item closed".into(), true);
        Ok(())
    }
}

pub struct WgcSource {
    shared: Arc<Shared>,
    control: Option<CaptureControl<Handler, String>>,
    kind: Kind,
    target: String,
    refresh_hz: f32,
}

fn backend(e: impl std::fmt::Display) -> CaptureError {
    CaptureError::Backend(e.to_string())
}

impl WgcSource {
    /// The primary monitor. Frames come only when the screen changes, so this does not wait.
    pub fn open_monitor(aux: Arc<AuxTap>) -> Result<Self, CaptureError> {
        let m = Monitor::primary().map_err(backend)?;
        let (w, h) = (m.width().map_err(backend)?, m.height().map_err(backend)?);
        let hz = m.refresh_rate().unwrap_or(0) as f32;
        let s = Self::start(Kind::WgcMonitor, m, "primary".into(), hz, aux)?;
        s.shared.width.store(w, Ordering::Relaxed);
        s.shared.height.store(h, Ordering::Relaxed);
        Ok(s)
    }

    /// The first window whose process matches `classes`. Waits up to 3 s for a first frame
    /// (it gives the size).
    pub fn open_window(classes: &[String], aux: Arc<AuxTap>) -> Result<Self, CaptureError> {
        let win = Window::enumerate()
            .map_err(backend)?
            .into_iter()
            .find(|w| {
                w.is_valid()
                    && w.title().is_ok_and(|t| !t.is_empty())
                    && w.process_name().is_ok_and(|n| exe_matches(&n, classes))
            })
            .ok_or_else(|| backend("no game window found (is the game running?)"))?;
        let title = win.title().unwrap_or_default();
        let hz = win
            .monitor()
            .and_then(|m| m.refresh_rate().ok())
            .unwrap_or(0) as f32;
        let s = Self::start(Kind::WgcWindow, win, title, hz, aux)?;
        let t0 = Instant::now();
        while s.shared.width.load(Ordering::Relaxed) == 0 {
            if t0.elapsed() > Duration::from_secs(3) {
                return Err(backend("no frame from the game window within 3 s"));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        Ok(s)
    }

    fn start<T>(
        kind: Kind,
        item: T,
        target: String,
        refresh_hz: f32,
        aux: Arc<AuxTap>,
    ) -> Result<Self, CaptureError>
    where
        T: TryInto<windows_capture::settings::GraphicsCaptureItemType> + Send + 'static,
    {
        let shared = Arc::new(Shared {
            ring: Ring::new(),
            width: AtomicU32::new(0),
            height: AtomicU32::new(0),
            pts_is_receive_time: AtomicBool::new(false),
            aux,
        });
        let settings = Settings::new(
            item,
            CursorCaptureSettings::WithoutCursor,
            // Asks for no yellow border; Windows ignores this before 11 or without permission.
            DrawBorderSettings::WithoutBorder,
            SecondaryWindowSettings::Default,
            MinimumUpdateIntervalSettings::Custom(Duration::ZERO),
            DirtyRegionSettings::Default,
            ColorFormat::Bgra8,
            shared.clone(),
        );
        let control = Handler::start_free_threaded(settings).map_err(backend)?;
        Ok(Self {
            shared,
            control: Some(control),
            kind,
            target,
            refresh_hz,
        })
    }

    pub fn stats(&self) -> RingStats {
        self.shared.ring.stats()
    }
}

impl FrameSource for WgcSource {
    fn next_frame(&mut self, timeout: Duration) -> Result<Option<FrameView<'_>>, CaptureError> {
        Ok(self.shared.ring.acquire(timeout)?.map(|s| s.view()))
    }

    fn set_region(&mut self, region: Rect) {
        self.shared.ring.set_region(region);
    }

    fn info(&self) -> SourceInfo {
        let s = &self.shared;
        SourceInfo {
            width: s.width.load(Ordering::Relaxed),
            height: s.height.load(Ordering::Relaxed),
            refresh_hz: self.refresh_hz,
            backend: self.kind.name().into(),
            monitor: self.target.clone(),
            scale: 1.0,
            pts_is_receive_time: s.pts_is_receive_time.load(Ordering::Relaxed),
        }
    }

    fn aux(&self) -> Option<Arc<AuxTap>> {
        Some(self.shared.aux.clone())
    }

    fn snapshot(&mut self) -> Result<OwnedFrame, CaptureError> {
        crate::ring::snapshot(&self.shared.ring, &self.info())
    }
}

impl Drop for WgcSource {
    fn drop(&mut self) {
        if let Some(c) = self.control.take() {
            let _ = c.stop();
        }
    }
}
