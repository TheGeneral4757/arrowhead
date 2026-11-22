//! Input latency test (spec 04). A test window (the CLI's terminal or the app's latency panel)
//! turns white the moment a key arrives; arrowhead sends a key through uinput and the screen
//! capture times when the white is seen. Send to seen (p50) is stored as the profile's input
//! latency, for reference only: it does not predict the game's offset (spec 04).

use std::sync::atomic::Ordering;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use arrowhead_core::config::{Profile, load_toml, save_toml};
use arrowhead_core::frame::{FrameSource, FrameView, Rect};
use arrowhead_core::input::{KeyAction, key_names};
use arrowhead_core::runtime::platform::EXIT;
use arrowhead_core::time::{Clock, Nanos, RealClock};
use serde::Serialize;

use crate::{Res, live_sink, load_config, profile_path};

#[derive(Clone, Debug, Serialize)]
pub struct LatencyResult {
    pub trials: u32,
    pub input_ms_p50: f64,
    pub input_ms_p90: f64,
    /// Key sent to key received by the test window, p50.
    pub recv_ms_p50: f64,
    /// The capture gives receive times, so the numbers include the capture delay.
    pub includes_capture: bool,
}

/// Mean brightness of every 16th pixel in both directions.
fn level(f: &FrameView) -> f64 {
    let (mut sum, mut n) = (0u64, 0u64);
    for y in (0..f.region.h as usize).step_by(16) {
        let row = &f.data[y * f.stride..];
        for x in (0..f.region.w as usize).step_by(16) {
            sum += row[x * 4] as u64 + row[x * 4 + 1] as u64 + row[x * 4 + 2] as u64;
            n += 1;
        }
    }
    sum as f64 / (3 * n.max(1)) as f64
}

/// Waits up to `ms` for a frame whose level is above (`bright`) or below the threshold.
fn wait_level(src: &mut dyn FrameSource, thr: f64, bright: bool, ms: u64) -> Res<Option<Nanos>> {
    let t0 = Instant::now();
    while t0.elapsed() < Duration::from_millis(ms) {
        if let Some(f) = src.next_frame(Duration::from_millis(50))?
            && (level(&f) > thr) == bright
        {
            return Ok(Some(f.pts));
        }
    }
    Ok(None)
}

fn settle(src: &mut dyn FrameSource, ms: u64) -> Res<f64> {
    let t0 = Instant::now();
    let mut l = 0.0;
    while t0.elapsed() < Duration::from_millis(ms) {
        if let Some(f) = src.next_frame(Duration::from_millis(50))? {
            l = level(&f);
        }
    }
    Ok(l)
}

/// p-th percentile (0-100) of an unsorted slice.
pub fn pct(v: &mut [f64], p: f64) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    v[((v.len() - 1) as f64 * p / 100.0).round() as usize]
}

/// `paint(white)` paints the test window; `keys` receives the time each key reached it.
pub fn run(
    src: &mut dyn FrameSource,
    trials: u32,
    paint: &dyn Fn(bool),
    keys: &Receiver<Nanos>,
) -> Res<LatencyResult> {
    let cfg = load_config()?;
    let info = src.info();
    src.set_region(Rect::new(0, 0, info.width, info.height));
    let mut sink = live_sink(key_names(cfg.keys.set, &cfg.keys.custom))?;

    // Levels with the window black and white: is it in the captured screen?
    paint(false);
    let black = settle(src, 400)?;
    paint(true);
    let white = settle(src, 400)?;
    paint(false);
    if white - black < 8.0 {
        return Err(format!(
            "the capture barely changes when the test window flashes (black {black:.0}, white {white:.0}): \
             make it big, on the captured monitor, and not covered"
        )
        .into());
    }
    let thr = (black + white) / 2.0;

    let clock = RealClock;
    let (mut seen_ms, mut recv_ms) = (Vec::new(), Vec::new());
    for i in 0..trials {
        if EXIT.load(Ordering::SeqCst) {
            break;
        }
        paint(false);
        if wait_level(src, thr, false, 1000)?.is_none() {
            continue;
        }
        settle(src, 150)?;
        while keys.try_recv().is_ok() {}
        let sent = clock.now();
        sink.send_batch(&[KeyAction::Press(0)])?;
        std::thread::sleep(Duration::from_millis(30));
        sink.send_batch(&[KeyAction::Release(0)])?;
        let Ok(recv) = keys.recv_timeout(Duration::from_millis(500)) else {
            sink.release_all()?;
            return Err(format!(
                "trial {}: the key did not reach the test window. Keep it focused and do not touch the keyboard",
                i + 1
            )
            .into());
        };
        recv_ms.push(recv.ms_since(sent));
        if let Some(pts) = wait_level(src, thr, true, 500)? {
            seen_ms.push(pts.ms_since(sent));
        }
    }
    sink.release_all()?;
    paint(false);

    if seen_ms.len() < 10 {
        return Err(format!("only {} of {trials} flashes were seen", seen_ms.len()).into());
    }
    let res = LatencyResult {
        trials: seen_ms.len() as u32,
        input_ms_p50: pct(&mut seen_ms, 50.0),
        input_ms_p90: pct(&mut seen_ms, 90.0),
        recv_ms_p50: pct(&mut recv_ms, 50.0),
        includes_capture: info.pts_is_receive_time,
    };
    let path = profile_path(&cfg).ok_or("no config dir")?;
    let mut prof: Profile = load_toml(&path)?;
    prof.latency.input_ms = res.input_ms_p50 as f32;
    save_toml(&path, &prof)?;
    Ok(res)
}
