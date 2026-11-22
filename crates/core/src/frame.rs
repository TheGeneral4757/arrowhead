use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use crate::time::Nanos;

/// Physical pixels of the captured stream.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    pub fn new(x: i32, y: i32, w: u32, h: u32) -> Self {
        Self { x, y, w, h }
    }

    /// Overlap of two rects, `None` if they do not touch.
    pub fn intersect(&self, o: &Rect) -> Option<Rect> {
        let x0 = self.x.max(o.x);
        let y0 = self.y.max(o.y);
        let x1 = (self.x + self.w as i32).min(o.x + o.w as i32);
        let y1 = (self.y + self.h as i32).min(o.y + o.h as i32);
        (x1 > x0 && y1 > y0).then(|| Rect::new(x0, y0, (x1 - x0) as u32, (y1 - y0) as u32))
    }
}

/// One frame of `region`, BGRx (4 bytes per pixel), rows `stride` bytes apart.
#[derive(Clone, Copy, Debug)]
pub struct FrameView<'a> {
    pub seq: u64,
    pub pts: Nanos,
    pub region: Rect,
    pub stride: usize,
    pub data: &'a [u8],
}

impl FrameView<'_> {
    /// (r, g, b) at stream coordinates. Panics outside `region`.
    #[inline(always)]
    pub fn px(&self, x: i32, y: i32) -> (u8, u8, u8) {
        let i = (y - self.region.y) as usize * self.stride + (x - self.region.x) as usize * 4;
        (self.data[i + 2], self.data[i + 1], self.data[i])
    }

    /// Tightly packed copy of `r`, which must lie inside `region`.
    pub fn crop(&self, r: Rect) -> OwnedFrame {
        let row = r.w as usize * 4;
        let x0 = (r.x - self.region.x) as usize * 4;
        let mut data = Vec::with_capacity(row * r.h as usize);
        for y in (r.y - self.region.y) as usize..(r.y - self.region.y) as usize + r.h as usize {
            let i = y * self.stride + x0;
            data.extend_from_slice(&self.data[i..i + row]);
        }
        OwnedFrame {
            seq: self.seq,
            pts: self.pts,
            region: r,
            data,
        }
    }

    /// Tightly packed copy (stride = w * 4).
    pub fn to_owned(&self) -> OwnedFrame {
        let row = self.region.w as usize * 4;
        let mut data = Vec::with_capacity(row * self.region.h as usize);
        for r in 0..self.region.h as usize {
            data.extend_from_slice(&self.data[r * self.stride..r * self.stride + row]);
        }
        OwnedFrame {
            seq: self.seq,
            pts: self.pts,
            region: self.region,
            data,
        }
    }
}

/// Tightly packed BGRx frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnedFrame {
    pub seq: u64,
    pub pts: Nanos,
    pub region: Rect,
    pub data: Vec<u8>,
}

impl OwnedFrame {
    pub fn view(&self) -> FrameView<'_> {
        FrameView {
            seq: self.seq,
            pts: self.pts,
            region: self.region,
            stride: self.region.w as usize * 4,
            data: &self.data,
        }
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct SourceInfo {
    pub width: u32,
    pub height: u32,
    pub refresh_hz: f32,
    pub backend: String,
    pub monitor: String,
    pub scale: f32,
    /// True when the backend gives no presentation time and `pts` is the receive time.
    pub pts_is_receive_time: bool,
}

#[derive(Debug)]
pub enum CaptureError {
    PermissionDenied,
    /// Monitor unplugged, mode change, stream ended. The runtime rebuilds the source.
    SourceLost,
    Backend(String),
}

impl std::fmt::Display for CaptureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PermissionDenied => f.write_str("screen capture permission denied"),
            Self::SourceLost => f.write_str("capture source lost"),
            Self::Backend(s) => write!(f, "capture backend: {s}"),
        }
    }
}

impl std::error::Error for CaptureError {}

/// Low-rate copies of small areas outside the main region, for the HUD reader (spec 03): the
/// stats panel, counts and scores lie outside the lane strips. The HUD thread asks for rects
/// with `request`; the capture thread copies them every `every`-th frame. The producer never
/// waits on it (`try_lock`, a busy tap skips that frame); the HUD thread waits in `wait`.
#[derive(Default)]
pub struct AuxTap {
    st: Mutex<AuxState>,
    cv: Condvar,
}

#[derive(Default)]
struct AuxState {
    rects: Vec<Rect>,
    every: u64,
    seq: u64,
    frames: Vec<OwnedFrame>,
}

impl AuxTap {
    /// Rects to copy (stream px, clipped by the producer) and how often. Empty = off.
    pub fn request(&self, rects: Vec<Rect>, every: u32) {
        let mut st = self.st.lock().unwrap_or_else(|e| e.into_inner());
        st.rects = rects;
        st.every = every.max(1) as u64;
    }

    /// Producer, once per captured frame `n`: the rects due now, or `None`. Never blocks.
    pub fn due(&self, n: u64) -> Option<Vec<Rect>> {
        let st = self.st.try_lock().ok()?;
        (!st.rects.is_empty() && n.is_multiple_of(st.every)).then(|| st.rects.clone())
    }

    /// Producer: the copies of the rects `due` returned, in the same order (a rect outside the
    /// stream is left out). Never blocks: a busy tap drops this set.
    pub fn deliver(&self, frames: Vec<OwnedFrame>) {
        if let Ok(mut st) = self.st.try_lock() {
            st.seq += 1;
            st.frames = frames;
            self.cv.notify_all();
        }
    }

    /// Producer helper for CPU backends: copies the due rects out of a full-stream view.
    pub fn copy_from(&self, n: u64, full: &FrameView) {
        if let Some(rects) = self.due(n) {
            let frames = rects
                .iter()
                .filter_map(|r| full.region.intersect(r))
                .map(|r| full.crop(r))
                .collect();
            self.deliver(frames);
        }
    }

    /// Consumer: the newest set after `after` (its number and copies), waiting up to `timeout`.
    pub fn wait(&self, after: u64, timeout: Duration) -> Option<(u64, Vec<OwnedFrame>)> {
        let st = self.st.lock().unwrap_or_else(|e| e.into_inner());
        let (st, _) = self
            .cv
            .wait_timeout_while(st, timeout, |s| s.seq <= after)
            .unwrap_or_else(|e| e.into_inner());
        (st.seq > after).then(|| (st.seq, st.frames.clone()))
    }
}

pub trait FrameSource: Send {
    /// Blocks up to `timeout`. Ok(None) on timeout. Frame borrows the ring slot until dropped.
    fn next_frame(&mut self, timeout: Duration) -> Result<Option<FrameView<'_>>, CaptureError>;
    /// Region to deliver, in stream pixels. Takes effect on the next frame.
    fn set_region(&mut self, region: Rect);
    /// Full stream size and refresh, for calibration and diagnostics.
    fn info(&self) -> SourceInfo;
    /// Grab one full-size frame (calibration, not hot path). May be slower.
    fn snapshot(&mut self) -> Result<OwnedFrame, CaptureError>;
    /// Side copies for the HUD reader; `None` when the backend has none (clips, tests).
    fn aux(&self) -> Option<Arc<AuxTap>> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intersect() {
        let a = Rect::new(0, 0, 10, 10);
        assert_eq!(
            a.intersect(&Rect::new(5, 5, 10, 10)),
            Some(Rect::new(5, 5, 5, 5))
        );
        assert_eq!(a.intersect(&Rect::new(10, 0, 5, 5)), None);
    }

    #[test]
    fn aux_tap_copies_due_rects() {
        let full = OwnedFrame {
            seq: 7,
            pts: Nanos(5),
            region: Rect::new(0, 0, 4, 2),
            data: (0..32).collect(),
        };
        let tap = AuxTap::default();
        tap.copy_from(0, &full.view());
        assert!(tap.wait(0, Duration::ZERO).is_none(), "nothing requested");
        tap.request(vec![Rect::new(1, 1, 2, 1), Rect::new(9, 9, 1, 1)], 2);
        tap.copy_from(1, &full.view());
        assert!(
            tap.wait(0, Duration::ZERO).is_none(),
            "not due on odd frames"
        );
        tap.copy_from(2, &full.view());
        let (n, fr) = tap.wait(0, Duration::ZERO).unwrap();
        assert_eq!(n, 1);
        assert_eq!(fr.len(), 1, "a rect outside the stream is left out");
        assert_eq!(fr[0].region, Rect::new(1, 1, 2, 1));
        assert_eq!(fr[0].data, (20..28).collect::<Vec<u8>>());
        assert_eq!((fr[0].seq, fr[0].pts), (7, Nanos(5)));
        assert!(tap.wait(n, Duration::from_millis(1)).is_none());
    }

    #[test]
    fn px_reads_bgrx() {
        let f = OwnedFrame {
            seq: 0,
            pts: Nanos(0),
            region: Rect::new(10, 20, 2, 1),
            data: vec![1, 2, 3, 0, 4, 5, 6, 0],
        };
        assert_eq!(f.view().px(11, 20), (6, 5, 4));
    }
}
