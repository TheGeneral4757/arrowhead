//! `run`: calibrate on the live screen, then play (spec 04, 05). Dry run unless `--live`.
//! The engine itself lives in `arrowhead-runner`, shared with the app; this prints its events.

use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use arrowhead_core::config::{Config, config_dir, load_toml};
use arrowhead_core::event::{EngineEvent, Level};
use arrowhead_core::frame::FrameSource;
use arrowhead_core::input::NullSink;
use arrowhead_core::plan::Cmd;
use arrowhead_core::runtime::platform::{install_exit_hooks, make_realtime};
use arrowhead_core::sched::{SchedEvent, Scheduler};
use arrowhead_core::time::{Clock, Nanos, RealClock};
use arrowhead_runner::{Control, Opts};

use crate::{Res, pct};

pub fn run(src: &mut dyn FrameSource, o: Opts) -> Res {
    // Ctrl+C stops play cleanly and releases every key.
    install_exit_hooks();
    let (tx, rx) = mpsc::sync_channel::<EngineEvent>(4096);
    let printer = std::thread::spawn(move || {
        let mut late = Vec::new();
        for e in rx {
            match e {
                EngineEvent::Log { level, text } => match level {
                    Level::Info => eprintln!("{text}"),
                    Level::Warn => eprintln!("WARNING: {text}"),
                    Level::Error => eprintln!("ERROR: {text}"),
                },
                EngineEvent::State {
                    state,
                    dry_run,
                    reason,
                } => {
                    eprintln!(
                        "state {state:?}{} ({reason})",
                        if dry_run { ", dry run" } else { "" }
                    )
                }
                EngineEvent::Hotkeys { bound: Some(b), .. } => eprintln!("hotkeys bound: {b}"),
                EngineEvent::Hotkeys { error: Some(e), .. } => {
                    eprintln!("WARNING: hotkeys off ({e}); Ctrl+C here or alt-tab away to stop")
                }
                EngineEvent::Focus { focused: Some(f) } => {
                    eprintln!("focus: {}", if f { "game" } else { "away" })
                }
                EngineEvent::Pressed {
                    planned_ms,
                    actual_ms,
                    ..
                } => late.push((actual_ms - planned_ms) * 1000.0),
                EngineEvent::Pipeline(p) => eprintln!(
                    "{:4.0} fps | scan p99 {:4.0} us | v {:>5.2} px/ms | presses {:4} | late-found missed {} | suppressed {}",
                    p.fps,
                    p.scan_us_p99,
                    p.speed_px_ms,
                    p.presses,
                    p.missed_by_engine,
                    p.suppressed
                ),
                _ => {}
            }
        }
        late
    });
    let ctl = Arc::new(Control::default());
    let sum = arrowhead_runner::run(src, o, ctl, tx);
    let mut late = printer.join().unwrap_or_default();
    let sum = sum.map_err(|e| -> Box<dyn std::error::Error> { e.to_string().into() })?;
    println!(
        "done: {} presses, {} missed by engine, {} suppressed, {} watchdog releases",
        sum.presses, sum.missed_by_engine, sum.suppressed, sum.watchdog
    );
    if !late.is_empty() {
        println!(
            "scheduler lateness us p50 {:.0} p99 {:.0}",
            pct(&mut late, 50.0),
            pct(&mut late, 99.0)
        );
    }
    Ok(())
}

/// `focus`: print the active window class as the KWin probe sees it (checks P5).
#[cfg(target_os = "linux")]
pub fn focus(seconds: f32) -> Res {
    use arrowhead_core::input::FocusProbe;
    let cfg: Config = config_dir()
        .map(|d| load_toml(&d.join("config.toml")))
        .transpose()?
        .unwrap_or_default();
    let mut probe =
        arrowhead_input::kwin::KwinProbe::start(cfg.safety.game_window_classes.clone())?;
    let t0 = Instant::now();
    let mut last = String::new();
    while t0.elapsed().as_secs_f32() < seconds {
        let class = probe.active_class();
        if class != last {
            println!(
                "{:6.1} s  active {class:?}  game focused: {:?}",
                t0.elapsed().as_secs_f32(),
                probe.game_focused().ok()
            );
            last = class;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub fn focus(_: f32) -> Res {
    Err("no focus probe on this OS yet".into())
}

/// `bench`: scheduler lateness on the real clock with a dry-run sink (spec 04: p99 < 0.3 ms).
pub fn bench(presses: u32) -> Res {
    eprintln!("bench thread: {}", make_realtime());
    let clock = RealClock;
    let mut s = Scheduler::new(15_000.0);
    let mut sink = NullSink;
    let mut ev = Vec::new();
    let t0 = clock.now();
    // Presses every 7-23 ms, like a dense stream, fed 32 ahead as the scan thread would.
    let mut at = t0.add_ms(20.0);
    let times: Vec<Nanos> = (0..presses)
        .map(|id| {
            at = at.add_ms(7.0 + (id * 7919 % 17) as f64);
            at
        })
        .collect();
    let mut late = Vec::new();
    let mut fed = 0;
    loop {
        while fed < times.len() && fed < late.len() + 32 {
            let at = times[fed];
            let id = fed as u32;
            s.submit(Cmd::Press {
                id,
                lane: (id % 4) as u8,
                at,
                release: at.add_ms(35.0),
            });
            fed += 1;
        }
        let Some(t) = s.next_due() else { break };
        clock.sleep_until(t);
        s.fire_due(clock.now(), true, &mut sink, &mut ev);
        for e in ev.drain(..) {
            if let SchedEvent::Pressed {
                planned, actual, ..
            } = e
            {
                late.push(actual.ms_since(planned) * 1000.0);
            }
        }
    }
    println!(
        "scheduler lateness over {} presses: p50 {:.1} us, p99 {:.1} us, max {:.1} us",
        late.len(),
        pct(&mut late, 50.0),
        pct(&mut late, 99.0),
        pct(&mut late, 100.0)
    );
    Ok(())
}
