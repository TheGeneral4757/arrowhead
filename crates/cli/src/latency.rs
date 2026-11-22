//! `latency`: input latency test (spec 04) with this terminal as the test window. The test
//! itself is `arrowhead_runner::latency`, shared with the app's Setup step.

use std::io::{Read, Write};
use std::process::Command;
use std::sync::mpsc;

use arrowhead_core::frame::FrameSource;
use arrowhead_core::runtime::platform::install_exit_hooks;
use arrowhead_core::time::{Clock, Nanos, RealClock};

use crate::Res;

const BLACK: &str = "\x1b[40m\x1b[2J\x1b[H";
const WHITE: &str = "\x1b[47m\x1b[2J\x1b[H";

fn paint(s: &str) {
    let mut o = std::io::stdout().lock();
    let _ = o.write_all(s.as_bytes());
    let _ = o.flush();
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
    let saved = Command::new("stty").arg("-g").output()?;
    let tty = TtyGuard(String::from_utf8_lossy(&saved.stdout).trim().to_string());
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
    let res = arrowhead_runner::latency::run(
        src,
        trials,
        &|white| paint(if white { WHITE } else { BLACK }),
        &rx,
    );
    drop(tty);
    let r = res.map_err(|e| -> Box<dyn std::error::Error> { e.to_string().into() })?;
    println!(
        "{} trials: key to terminal p50 {:.1} ms; key to seen on screen p50 {:.1} ms, p90 {:.1} ms",
        r.trials, r.recv_ms_p50, r.input_ms_p50, r.input_ms_p90
    );
    if r.includes_capture {
        println!(
            "note: this capture gives receive times, so the number includes the capture delay"
        );
    }
    println!(
        "saved input latency {:.1} ms to the profile (for reference: `run` uses latency.game_bias_ms)",
        r.input_ms_p50
    );
    Ok(())
}
