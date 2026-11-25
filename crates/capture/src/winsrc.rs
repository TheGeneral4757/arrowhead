//! The Windows `FrameSource` the apps use: picks DXGI / WGC by `capture.windows_backend`
//! and, in `auto`, moves down the list when a backend fails (spec 02).

use std::time::Duration;

use arrowhead_core::config::WindowsBackend;
use std::sync::Arc;

use arrowhead_core::frame::{
    AuxTap, CaptureError, FrameSource, FrameView, OwnedFrame, Rect, SourceInfo,
};

use crate::dxgi::DxgiSource;
use crate::ring::RingStats;
use crate::wgc::WgcSource;
use crate::winpolicy::{Kind, backend_order};

enum Inner {
    Dxgi(DxgiSource),
    Wgc(WgcSource),
}

pub struct WindowsSource {
    order: &'static [Kind],
    idx: usize,
    inner: Inner,
    region: Rect,
    classes: Vec<String>,
    /// Handed to every inner backend, so the HUD thread's handle survives a fallback.
    aux: Arc<AuxTap>,
}

fn open_kind(kind: Kind, classes: &[String], aux: &Arc<AuxTap>) -> Result<Inner, CaptureError> {
    Ok(match kind {
        Kind::Dxgi => Inner::Dxgi(DxgiSource::open(aux.clone())?),
        Kind::WgcMonitor => Inner::Wgc(WgcSource::open_monitor(aux.clone())?),
        Kind::WgcWindow => Inner::Wgc(WgcSource::open_window(classes, aux.clone())?),
    })
}

impl WindowsSource {
    /// Opens the first backend of the setting's order that starts. `classes` is
    /// `safety.game_window_classes` (used to find the window for WGC window).
    pub fn open(backend: WindowsBackend, classes: Vec<String>) -> Result<Self, CaptureError> {
        let order = backend_order(backend);
        let aux = Arc::new(AuxTap::default());
        let mut last = None;
        for (idx, &kind) in order.iter().enumerate() {
            match open_kind(kind, &classes, &aux) {
                Ok(inner) => {
                    return Ok(Self {
                        order,
                        idx,
                        inner,
                        region: Rect::default(),
                        classes,
                        aux,
                    });
                }
                Err(CaptureError::PermissionDenied) => return Err(CaptureError::PermissionDenied),
                Err(e) => {
                    eprintln!("capture: {} did not start ({e})", kind.name());
                    last = Some(e);
                }
            }
        }
        Err(last.unwrap_or_else(|| CaptureError::Backend("no capture backend".into())))
    }

    pub fn stats(&self) -> RingStats {
        match &self.inner {
            Inner::Dxgi(s) => s.stats(),
            Inner::Wgc(s) => s.stats(),
        }
    }

    fn src(&mut self) -> &mut dyn FrameSource {
        match &mut self.inner {
            Inner::Dxgi(s) => s,
            Inner::Wgc(s) => s,
        }
    }

    /// Same as `src` as a raw pointer that does not keep `self` borrowed.
    fn src_ptr(&mut self) -> *mut dyn FrameSource {
        match &mut self.inner {
            Inner::Dxgi(s) => s as *mut DxgiSource as *mut dyn FrameSource,
            Inner::Wgc(s) => s as *mut WgcSource as *mut dyn FrameSource,
        }
    }

    /// Replaces a failed backend with the next one that starts. False when none is left.
    fn fall_back(&mut self, why: &CaptureError) -> bool {
        while self.idx + 1 < self.order.len() {
            self.idx += 1;
            let kind = self.order[self.idx];
            eprintln!(
                "capture: {} failed ({why}); trying {}",
                self.order[self.idx - 1].name(),
                kind.name()
            );
            if let Ok(mut inner) = open_kind(kind, &self.classes, &self.aux) {
                match &mut inner {
                    Inner::Dxgi(s) => s.set_region(self.region),
                    Inner::Wgc(s) => s.set_region(self.region),
                }
                self.inner = inner;
                return true;
            }
        }
        false
    }
}

impl FrameSource for WindowsSource {
    fn next_frame(&mut self, timeout: Duration) -> Result<Option<FrameView<'_>>, CaptureError> {
        let p = self.src_ptr();
        // SAFETY: `p` points into `self.inner`. The Ok arm returns the frame borrowed from it
        // for the caller's `&mut self` lifetime; the Err arm holds no reference into the source,
        // so replacing `self.inner` there is sound (the borrow checker cannot see that split).
        match unsafe { &mut *p }.next_frame(timeout) {
            Ok(f) => Ok(f),
            // Lost (mode change, window closed): the runtime rebuilds the source from the top.
            Err(e @ (CaptureError::PermissionDenied | CaptureError::SourceLost)) => Err(e),
            Err(e) if self.fall_back(&e) => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn set_region(&mut self, region: Rect) {
        self.region = region;
        self.src().set_region(region);
    }

    fn info(&self) -> SourceInfo {
        match &self.inner {
            Inner::Dxgi(s) => s.info(),
            Inner::Wgc(s) => s.info(),
        }
    }

    fn aux(&self) -> Option<Arc<AuxTap>> {
        Some(self.aux.clone())
    }

    fn snapshot(&mut self) -> Result<OwnedFrame, CaptureError> {
        self.src().snapshot()
    }
}
