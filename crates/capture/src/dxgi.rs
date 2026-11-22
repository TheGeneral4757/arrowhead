//! Windows capture: DXGI Desktop Duplication (spec 02). Ported from fnf-thing-v2
//! `scanner/dxgi.rs`. A capture thread copies only the region into a staging texture,
//! maps it into the frame ring, and stamps `DXGI_OUTDUPL_FRAME_INFO.LastPresentTime` (QPC)
//! on the same clock as `Clock::now()`.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use arrowhead_core::frame::{
    AuxTap, CaptureError, FrameSource, FrameView, OwnedFrame, Rect, SourceInfo,
};
use arrowhead_core::runtime::platform::{make_realtime, qpc_ticks_to_ns};
use arrowhead_core::time::Nanos;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BOX, D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ,
    D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, DXGI_ERROR_ACCESS_LOST, DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO,
    IDXGIAdapter, IDXGIAdapter1, IDXGIFactory1, IDXGIOutput1, IDXGIOutputDuplication,
    IDXGIResource,
};
use windows::core::Interface;

use crate::ring::{Ring, RingStats};
use crate::winpolicy::FailureWindow;

/// Give up (and let `auto` fall back to WGC) after this many failures in this many ms.
const FAIL_LIMIT: usize = 3;
const FAIL_SPAN_MS: u64 = 5000;
/// Only bounds how often the thread checks `stop`; `AcquireNextFrame` returns on a new frame.
const ACQUIRE_TIMEOUT_MS: u32 = 50;

struct Shared {
    ring: Ring,
    width: AtomicU32,
    height: AtomicU32,
    /// Refresh rate x 1000.
    refresh_milli: AtomicU32,
}

pub struct DxgiSource {
    aux: Arc<AuxTap>,
    shared: Arc<Shared>,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

fn backend(e: impl std::fmt::Display) -> CaptureError {
    CaptureError::Backend(e.to_string())
}

/// What went wrong in the capture loop, so the right error reaches the ring.
enum Fault {
    /// Counts toward the 3-in-5-s limit; the duplication is rebuilt and the loop goes on.
    Retry(String),
    /// Not worth retrying (for example an HDR desktop format).
    Fatal(String),
}

struct Capturer {
    _adapter: IDXGIAdapter1,
    output: IDXGIOutput1,
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    dup: Option<IDXGIOutputDuplication>,
    staging: Option<(ID3D11Texture2D, u32, u32)>,
    /// Staging textures for the HUD side copies, one per size.
    aux_staging: Vec<(ID3D11Texture2D, u32, u32)>,
    aux: Arc<AuxTap>,
    frames: u64,
    width: u32,
    height: u32,
    refresh_milli: u32,
}

/// The output at (0, 0) (the primary monitor), else the first one found.
fn find_output() -> Result<(IDXGIAdapter1, IDXGIOutput1), CaptureError> {
    // SAFETY: plain DXGI enumeration.
    unsafe {
        let factory: IDXGIFactory1 = CreateDXGIFactory1().map_err(backend)?;
        let mut first = None;
        let mut i = 0;
        while let Ok(adapter) = factory.EnumAdapters1(i) {
            i += 1;
            let mut j = 0;
            while let Ok(output) = adapter.EnumOutputs(j) {
                j += 1;
                let Ok(output1) = output.cast::<IDXGIOutput1>() else {
                    continue;
                };
                if let Ok(d) = output1.GetDesc() {
                    let r = d.DesktopCoordinates;
                    if r.left <= 0 && r.top <= 0 && r.right > 0 && r.bottom > 0 {
                        return Ok((adapter, output1));
                    }
                }
                first.get_or_insert((adapter.clone(), output1));
            }
        }
        first.ok_or_else(|| backend("no DXGI output found"))
    }
}

impl Capturer {
    fn new(aux: Arc<AuxTap>) -> Result<Self, CaptureError> {
        let (adapter, output) = find_output()?;
        let (mut device, mut context) = (None, None);
        // SAFETY: out pointers are valid. The device must come from the output's own adapter,
        // otherwise duplication fails on hybrid-GPU laptops.
        unsafe {
            D3D11CreateDevice(
                &adapter.cast::<IDXGIAdapter>().map_err(backend)?,
                D3D_DRIVER_TYPE_UNKNOWN,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&[D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0]),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
            .map_err(|e| backend(format!("D3D11CreateDevice: {e}")))?;
        }
        let (Some(device), Some(context)) = (device, context) else {
            return Err(backend("D3D11CreateDevice returned no device"));
        };
        let mut c = Self {
            _adapter: adapter,
            output,
            device,
            context,
            dup: None,
            staging: None,
            aux_staging: Vec::new(),
            aux,
            frames: 0,
            width: 0,
            height: 0,
            refresh_milli: 0,
        };
        c.duplicate()?;
        Ok(c)
    }

    /// (Re)creates the duplication and records the output's size and refresh rate.
    fn duplicate(&mut self) -> Result<(), CaptureError> {
        // SAFETY: valid device; GetDesc only fills a struct.
        let (dup, desc) = unsafe {
            let dup = self
                .output
                .DuplicateOutput(&self.device)
                .map_err(|e| backend(format!("DuplicateOutput: {e}")))?;
            let desc = dup.GetDesc();
            (dup, desc)
        };
        let r = desc.ModeDesc.RefreshRate;
        let milli = if r.Denominator == 0 {
            0
        } else {
            (r.Numerator as u64 * 1000 / r.Denominator as u64) as u32
        };
        self.dup = Some(dup);
        self.width = desc.ModeDesc.Width;
        self.height = desc.ModeDesc.Height;
        self.refresh_milli = milli;
        Ok(())
    }

    fn staging_for(&mut self, w: u32, h: u32) -> Result<&ID3D11Texture2D, Fault> {
        if !matches!(&self.staging, Some((_, sw, sh)) if *sw == w && *sh == h) {
            let desc = D3D11_TEXTURE2D_DESC {
                Width: w,
                Height: h,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_STAGING,
                BindFlags: 0,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                MiscFlags: 0,
            };
            let mut tex = None;
            // SAFETY: valid desc and out pointer.
            unsafe { self.device.CreateTexture2D(&desc, None, Some(&mut tex)) }
                .map_err(|e| Fault::Retry(format!("CreateTexture2D: {e}")))?;
            let tex = tex.ok_or_else(|| Fault::Retry("CreateTexture2D gave no texture".into()))?;
            self.staging = Some((tex, w, h));
        }
        Ok(&self.staging.as_ref().unwrap().0)
    }

    /// Copies the requested region of the acquired desktop image into the ring.
    fn deliver(&mut self, ring: &Ring, res: &IDXGIResource, present_qpc: i64) -> Result<(), Fault> {
        let tex: ID3D11Texture2D = res.cast().map_err(|e| Fault::Retry(e.to_string()))?;
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        // SAFETY: fills a struct.
        unsafe { tex.GetDesc(&mut desc) };
        if desc.Format != DXGI_FORMAT_B8G8R8A8_UNORM {
            return Err(Fault::Fatal(format!(
                "desktop format {:?} is not BGRA8 (HDR or 10-bit?)",
                desc.Format
            )));
        }
        let (sw, sh) = (desc.Width, desc.Height);
        let pts = Nanos(qpc_ticks_to_ns(present_qpc));
        let mut fault = None;
        ring.produce(|slot, want| {
            let full = want.w == 0 || want.h == 0;
            let want = if full { Rect::new(0, 0, sw, sh) } else { want };
            let Some(r) = want.intersect(&Rect::new(0, 0, sw, sh)) else {
                return false;
            };
            let staging = match self.staging_for(r.w, r.h) {
                Ok(s) => s.clone(),
                Err(f) => {
                    fault = Some(f);
                    return false;
                }
            };
            let b = D3D11_BOX {
                left: r.x as u32,
                top: r.y as u32,
                front: 0,
                right: r.x as u32 + r.w,
                bottom: r.y as u32 + r.h,
                back: 1,
            };
            let mut m = D3D11_MAPPED_SUBRESOURCE::default();
            // SAFETY: the box lies inside the source texture; the mapped memory is valid
            // for `RowPitch * h` bytes until Unmap, and is only read inside this block.
            unsafe {
                self.context
                    .CopySubresourceRegion(&staging, 0, 0, 0, 0, &tex, 0, Some(&b));
                if let Err(e) = self
                    .context
                    .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut m))
                {
                    fault = Some(Fault::Retry(format!("Map: {e}")));
                    return false;
                }
                let row = r.w as usize * 4;
                slot.data.resize(row * r.h as usize, 0);
                let src = m.pData as *const u8;
                for y in 0..r.h as usize {
                    let line = std::slice::from_raw_parts(src.add(y * m.RowPitch as usize), row);
                    slot.data[y * row..(y + 1) * row].copy_from_slice(line);
                }
                self.context.Unmap(&staging, 0);
            }
            slot.region = r;
            slot.stride = r.w as usize * 4;
            slot.pts = pts;
            true
        });
        self.frames += 1;
        if let Some(rects) = self.aux.due(self.frames) {
            // A failed side copy must not take the capture down: the HUD just misses a set.
            if let Err(e) = self.copy_aux(&tex, &rects, (sw, sh), self.frames, pts) {
                eprintln!("capture: aux copy: {e}");
            }
        }
        fault.map_or(Ok(()), Err)
    }

    /// Copies each due rect (clipped to the desktop) into a cached staging texture and maps it.
    fn copy_aux(
        &mut self,
        tex: &ID3D11Texture2D,
        rects: &[Rect],
        (sw, sh): (u32, u32),
        seq: u64,
        pts: Nanos,
    ) -> Result<(), String> {
        let desktop = Rect::new(0, 0, sw, sh);
        let mut out = Vec::with_capacity(rects.len());
        for r in rects.iter().filter_map(|r| r.intersect(&desktop)) {
            let i = match self
                .aux_staging
                .iter()
                .position(|(_, w, h)| (*w, *h) == (r.w, r.h))
            {
                Some(i) => i,
                None => {
                    let desc = D3D11_TEXTURE2D_DESC {
                        Width: r.w,
                        Height: r.h,
                        MipLevels: 1,
                        ArraySize: 1,
                        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                        SampleDesc: DXGI_SAMPLE_DESC {
                            Count: 1,
                            Quality: 0,
                        },
                        Usage: D3D11_USAGE_STAGING,
                        BindFlags: 0,
                        CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                        MiscFlags: 0,
                    };
                    let mut t = None;
                    // SAFETY: valid desc and out pointer.
                    unsafe { self.device.CreateTexture2D(&desc, None, Some(&mut t)) }
                        .map_err(|e| format!("CreateTexture2D: {e}"))?;
                    let t = t.ok_or("CreateTexture2D gave no texture")?;
                    self.aux_staging.push((t, r.w, r.h));
                    self.aux_staging.len() - 1
                }
            };
            let staging = &self.aux_staging[i].0;
            let b = D3D11_BOX {
                left: r.x as u32,
                top: r.y as u32,
                front: 0,
                right: r.x as u32 + r.w,
                bottom: r.y as u32 + r.h,
                back: 1,
            };
            let mut m = D3D11_MAPPED_SUBRESOURCE::default();
            let row = r.w as usize * 4;
            let mut data = vec![0u8; row * r.h as usize];
            // SAFETY: the box is inside the desktop texture; the mapped memory is valid for
            // `RowPitch * h` bytes until Unmap and only read inside this block.
            unsafe {
                self.context
                    .CopySubresourceRegion(staging, 0, 0, 0, 0, tex, 0, Some(&b));
                self.context
                    .Map(staging, 0, D3D11_MAP_READ, 0, Some(&mut m))
                    .map_err(|e| format!("Map: {e}"))?;
                let src = m.pData as *const u8;
                for y in 0..r.h as usize {
                    let line = std::slice::from_raw_parts(src.add(y * m.RowPitch as usize), row);
                    data[y * row..(y + 1) * row].copy_from_slice(line);
                }
                self.context.Unmap(staging, 0);
            }
            out.push(OwnedFrame {
                seq,
                pts,
                region: r,
                data,
            });
        }
        self.aux.deliver(out);
        Ok(())
    }

    /// One pass of the loop: wait for a frame, deliver it, release it.
    fn step(&mut self, ring: &Ring) -> Result<(), Fault> {
        let Some(dup) = self.dup.clone() else {
            return Err(Fault::Retry("no duplication".into()));
        };
        let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut res: Option<IDXGIResource> = None;
        // SAFETY: valid out pointers; the frame is released below.
        match unsafe { dup.AcquireNextFrame(ACQUIRE_TIMEOUT_MS, &mut info, &mut res) } {
            Ok(()) => {}
            Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => return Ok(()),
            Err(e) if e.code() == DXGI_ERROR_ACCESS_LOST => {
                self.dup = None;
                return Err(Fault::Retry("ACCESS_LOST".into()));
            }
            Err(e) => return Err(Fault::Retry(format!("AcquireNextFrame: {e}"))),
        }
        // LastPresentTime 0 means only the mouse moved: the image is the one we have.
        let out = match (&res, info.LastPresentTime) {
            (Some(r), t) if t != 0 => self.deliver(ring, r, t),
            _ => Ok(()),
        };
        // SAFETY: a frame was acquired above.
        unsafe {
            let _ = dup.ReleaseFrame();
        }
        out
    }
}

fn run(mut cap: Capturer, shared: &Shared, stop: &AtomicBool, (w0, h0): (u32, u32)) {
    let t0 = Instant::now();
    let mut fails = FailureWindow::new(FAIL_LIMIT, FAIL_SPAN_MS);
    while !stop.load(Ordering::Relaxed) {
        let fault = match cap.step(&shared.ring) {
            Ok(()) => continue,
            Err(f) => f,
        };
        match fault {
            Fault::Fatal(m) => return shared.ring.close(m, false),
            Fault::Retry(m) => {
                if fails.record(t0.elapsed().as_millis() as u64) {
                    return shared
                        .ring
                        .close(format!("{FAIL_LIMIT} failures in 5 s, last: {m}"), false);
                }
                std::thread::sleep(Duration::from_millis(100));
                match cap.duplicate() {
                    Ok(()) if (cap.width, cap.height) != (w0, h0) => {
                        let m = format!("display mode changed to {}x{}", cap.width, cap.height);
                        return shared.ring.close(m, true);
                    }
                    Ok(()) => {}
                    // The next step reports "no duplication" and counts again.
                    Err(_) => cap.dup = None,
                }
            }
        }
    }
}

impl DxgiSource {
    /// Starts duplicating the primary monitor and waits for the device and duplication.
    pub fn open(aux: Arc<AuxTap>) -> Result<Self, CaptureError> {
        let shared = Arc::new(Shared {
            ring: Ring::new(),
            width: AtomicU32::new(0),
            height: AtomicU32::new(0),
            refresh_milli: AtomicU32::new(0),
        });
        let stop = Arc::new(AtomicBool::new(false));
        let aux2 = aux.clone();
        let (tx, rx) = mpsc::channel();
        let (s2, st2) = (shared.clone(), stop.clone());
        let join = std::thread::Builder::new()
            .name("dxgi-capture".into())
            .spawn(move || {
                make_realtime();
                // D3D objects are created on this thread and never leave it.
                let cap = match Capturer::new(aux) {
                    Ok(c) => c,
                    Err(e) => return drop(tx.send(Err(e))),
                };
                let size = (cap.width, cap.height);
                s2.width.store(size.0, Ordering::Relaxed);
                s2.height.store(size.1, Ordering::Relaxed);
                s2.refresh_milli.store(cap.refresh_milli, Ordering::Relaxed);
                let _ = tx.send(Ok(()));
                run(cap, &s2, &st2, size);
            })
            .map_err(backend)?;
        match rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(())) => Ok(Self {
                aux: aux2,
                shared,
                stop,
                join: Some(join),
            }),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(backend("DXGI did not start within 5 s")),
        }
    }

    pub fn stats(&self) -> RingStats {
        self.shared.ring.stats()
    }
}

impl FrameSource for DxgiSource {
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
            refresh_hz: s.refresh_milli.load(Ordering::Relaxed) as f32 / 1000.0,
            backend: "dxgi".into(),
            monitor: "primary".into(),
            scale: 1.0,
            pts_is_receive_time: false,
        }
    }

    fn aux(&self) -> Option<Arc<AuxTap>> {
        Some(self.aux.clone())
    }

    fn snapshot(&mut self) -> Result<OwnedFrame, CaptureError> {
        crate::ring::snapshot(&self.shared.ring, &self.info())
    }
}

impl Drop for DxgiSource {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}
