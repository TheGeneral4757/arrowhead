//! `latency`: input latency test (spec 04). This terminal is the test window: arrowhead sends
//! a key through uinput, the terminal turns white when the key arrives, and the screen capture
//! times when the white is seen. Send to seen (p50) is stored as the profile's input latency,
//! for reference only: it does not predict the game's offset (spec 04).

use std::io::{Read, Write};
use std::process::Command;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::time::Duration;

use arrowhead_core::config::{Config, Profile, config_dir, load_toml, save_toml};
use arrowhead_core::frame::{FrameSource, FrameView, Rect};
use arrowhead_core::input::{KeyAction, key_names};
use arrowhead_core::runtime::platform::{EXIT, install_exit_hooks};
use arrowhead_core::time::{Clock, Nanos, RealClock};

use crate::run::live_sink;
use crate::{Res, pct};

const BLACK: &str = "\x1b[40m\x1b[2J\x1b[H";
const WHITE: &str = "\x1b[47m\x1b[2J\x1b[H";

fn paint(s: &str) {
    let mut o = std::io::stdout().lock();
    let _ = o.write_all(s.as_bytes());
    let _ = o.flush();
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
fn wait_level(
    src: &mut dyn FrameSource,
    thr: f64,
    bright: bool,
    ms: u64,
) -> Result<Option<Nanos>, Box<dyn std::error::Error>> {
    let t0 = std::time::Instant::now();
    while t0.elapsed() < Duration::from_millis(ms) {
        if let Some(f) = src.next_frame(Duration::from_millis(50))?
            && (level(&f) > thr) == bright
        {
            return Ok(Some(f.pts));
        }
    }
    Ok(None)
}

fn settle(src: &mut dyn FrameSource, ms: u64) -> Result<f64, Box<dyn std::error::Error>> {
    let t0 = std::time::Instant::now();
    let mut l = 0.0;
    while t0.elapsed() < Duration::from_millis(ms) {
        if let Some(f) = src.next_frame(Duration::from_millis(50))? {
            l = level(&f);
        }
    }
    Ok(l)
}

struct TtyGuard(String);

impl Drop for TtyGuard {
    fn drop(&mut self) {
        let _ = Command::new("stty").arg(&self.0).status();
        paint("\x1b[0m\x1b[2J\x1b[H");
    }
}

pub fn latency(src: &mut dyn FrameSource, trials: u32) -> Res {
    install_exit_hooks();
    let cfg: Config = config_dir()
        .map(|d| load_toml(&d.join("config.toml")))
        .transpose()?
        .unwrap_or_default();
    let info = src.info();
    src.set_region(Rect::new(0, 0, info.width, info.height));
    let keys = key_names(cfg.keys.set, &cfg.keys.custom);
    let mut sink = live_sink(keys)?;

    let saved = Command::new("stty").arg("-g").output()?;
    let _tty = TtyGuard(String::from_utf8_lossy(&saved.stdout).trim().to_string());
    Command::new("stty")
        .args(["-icanon", "-echo", "min", "1"])
        .status()?;

    // The key turns the terminal white the moment it arrives, before the main thread hears of it.
    let (tx, rx) = mpsc::channel::<Nanos>();
    std::thread::spawn(move || {
        let mut b = [0u8; 64];
        while let Ok(n) = std::io::stdin().read(&mut b) {
            if n == 0 {
                return;
            }
            paint(WHITE);
            if tx.send(RealClock.now()).is_err() {
                return;
            }
        }
    });

    // Levels with the terminal black and white: is this terminal in the captured screen?
    paint(BLACK);
    let black = settle(src, 400)?;
    paint(WHITE);
    let white = settle(src, 400)?;
    paint(BLACK);
    if white - black < 8.0 {
        return Err(format!(
            "the capture barely changes when this terminal flashes (black {black:.0}, white {white:.0}): \
             make the terminal big, on the captured monitor, and not covered"
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
        paint(BLACK);
        if wait_level(src, thr, false, 1000)?.is_none() {
            continue;
        }
        settle(src, 150)?;
        while rx.try_recv().is_ok() {}
        let sent = clock.now();
        sink.send_batch(&[KeyAction::Press(0)])?;
        std::thread::sleep(Duration::from_millis(30));
        sink.send_batch(&[KeyAction::Release(0)])?;
        let Ok(recv) = rx.recv_timeout(Duration::from_millis(500)) else {
            sink.release_all()?;
            return Err(format!(
                "trial {}: the key did not reach this terminal. Keep it focused and do not touch the keyboard",
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
    drop(_tty);

    if seen_ms.len() < 10 {
        return Err(format!("only {} of {trials} flashes were seen", seen_ms.len()).into());
    }
    let input_ms = pct(&mut seen_ms.clone(), 50.0);
    println!(
        "{} trials: key to terminal p50 {:.1} ms; key to seen on screen p50 {:.1} ms, p90 {:.1} ms",
        seen_ms.len(),
        pct(&mut recv_ms, 50.0),
        input_ms,
        pct(&mut seen_ms, 90.0)
    );
    if info.pts_is_receive_time {
        println!(
            "note: this capture gives receive times, so the number includes the capture delay"
        );
    }
    let path = config_dir()
        .ok_or("no config dir")?
        .join("profiles")
        .join(format!("{}.toml", cfg.profile));
    let mut prof: Profile = load_toml(&path)?;
    prof.latency.input_ms = input_ms as f32;
    save_toml(&path, &prof)?;
    println!(
        "saved input latency {input_ms:.1} ms to {} (for reference: `run` uses latency.game_bias_ms)",
        path.display()
    );
    Ok(())
}
