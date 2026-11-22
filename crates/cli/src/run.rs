//! `run`: calibrate on the live screen, then play (spec 04, 05). Dry run unless `--live`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use arrowhead_core::config::{Config, Profile, Scroll, Side, config_dir, load_toml, save_toml};
use arrowhead_core::detect::calib::{
    Blob, ColourLearner, Group, find_blobs, find_groups, guess_scroll, layout_for, pick_ours,
};
use arrowhead_core::detect::{Colours, DetectConfig, Detector, Layout};
use arrowhead_core::engine::Engine;
use arrowhead_core::frame::{FrameSource, FrameView, OwnedFrame};
use arrowhead_core::input::{KeySink, NullSink, key_names};
use arrowhead_core::plan::{Cmd, PlanConfig};
use arrowhead_core::runtime::platform::make_realtime;
use arrowhead_core::sched::{SchedEvent, Scheduler};
use arrowhead_core::time::{Clock, Nanos, RealClock};

use crate::{Res, pct};

pub struct Opts {
    pub live: bool,
    pub seconds: f32,
    pub offset_ms: f64,
    pub learn_s: f32,
    pub relearn: bool,
}

fn next(src: &mut dyn FrameSource) -> Result<OwnedFrame, Box<dyn std::error::Error>> {
    loop {
        if let Some(f) = src.next_frame(Duration::from_millis(500))? {
            return Ok(f.to_owned());
        }
    }
}

/// Background median from frames spread over `secs` (spec 03, colour learning).
fn bg_learner(
    src: &mut dyn FrameSource,
    layout: &Layout,
    cfg: &DetectConfig,
    secs: f32,
) -> Result<ColourLearner, Box<dyn std::error::Error>> {
    src.set_region(layout.region(cfg));
    let mut bg = Vec::new();
    let step = Duration::from_secs_f32(secs / 20.0);
    for _ in 0..21 {
        let t = Instant::now();
        bg.push(next(src)?);
        std::thread::sleep(step.saturating_sub(t.elapsed()));
    }
    let views: Vec<FrameView> = bg.iter().map(|f| f.view()).collect();
    Ok(ColourLearner::new(layout.clone(), cfg.clone(), &views))
}

fn feed(
    src: &mut dyn FrameSource,
    learner: &mut ColourLearner,
    secs: f32,
) -> Result<(), Box<dyn std::error::Error>> {
    let t0 = Instant::now();
    while t0.elapsed().as_secs_f32() < secs {
        learner.feed(&next(src)?.view());
    }
    Ok(())
}

/// Background frames (spread over `secs`), then colour learning on live frames for `secs`.
fn learn(
    src: &mut dyn FrameSource,
    layout: &Layout,
    cfg: &DetectConfig,
    secs: f32,
) -> Result<ColourLearner, Box<dyn std::error::Error>> {
    let mut learner = bg_learner(src, layout, cfg, secs)?;
    feed(src, &mut learner, secs)?;
    // Jacks merge into long runs that do not count, so dense songs need longer: up to 30 s more.
    let t0 = Instant::now();
    while let Err(e) = learner.finish() {
        if t0.elapsed().as_secs() >= 30 {
            return Err(e.into());
        }
        feed(src, &mut learner, 1.0)?;
    }
    Ok(learner)
}

fn saved_group(p: &Profile) -> Group {
    let d = 2.0 * p.radius_px;
    // shortcut: assumes receptors brighter than their surround (grey on a dark field); store the flag in the profile if a skin breaks this.
    let blobs = p.receptors.map(|[cx, cy]| Blob {
        cx,
        cy,
        diam: d,
        size: d,
        bright: true,
    });
    Group {
        blobs,
        cx: p.receptors.iter().map(|r| r[0]).sum::<f32>() / 4.0,
        cy: p.receptors.iter().map(|r| r[1]).sum::<f32>() / 4.0,
        diam: d,
        pitch: (p.receptors[3][0] - p.receptors[0][0]).abs() / 3.0,
    }
}

fn save_profile(path: Option<&std::path::Path>, prof: &Profile) {
    if let Some(p) = path {
        match save_toml(p, prof) {
            Ok(()) => eprintln!("saved colours to {}", p.display()),
            Err(e) => eprintln!("could not save the profile: {e}"),
        }
    }
}

pub fn run(src: &mut dyn FrameSource, o: Opts) -> Res {
    let cfg: Config = config_dir()
        .map(|d| load_toml(&d.join("config.toml")))
        .transpose()?
        .unwrap_or_default();
    let info = src.info();
    let dcfg = DetectConfig::default();
    eprintln!(
        "stream {}x{} @ {:.0} Hz ({}). Calibrating: have a song playing, keep the game visible.",
        info.width, info.height, info.refresh_hz, info.backend
    );

    let stream = [info.width, info.height];
    let prof_path = config_dir().map(|d| d.join("profiles").join(format!("{}.toml", cfg.profile)));
    let mut prof: Profile = prof_path
        .as_deref()
        .map(load_toml)
        .transpose()?
        .unwrap_or_default();
    let same_display = prof.matches(&info.monitor, stream, info.scale) && prof.radius_px > 0.0;
    // Receptors and colours do not change between songs: with a saved layout only the stage
    // background (1.5 s; the detector keeps adapting it) is learned before play.
    let saved = match (&prof.layout, &prof.colours) {
        (Some(l), Some(c)) if same_display && !o.relearn => Some((l.clone(), c.clone())),
        _ => None,
    };
    let (layout, learner, colours) = if let Some((layout, c)) = saved {
        eprintln!(
            "using the saved layout and colours (--relearn if the game window moved or the skin changed)"
        );
        (layout.clone(), bg_learner(src, &layout, &dcfg, 1.5)?, c)
    } else {
        // 25 snapshots over 6 s: in dense charts notes cover the receptors most of the time.
        let mut snaps = Vec::new();
        for _ in 0..25 {
            snaps.push(src.snapshot()?);
            std::thread::sleep(Duration::from_millis(250));
        }
        let views: Vec<FrameView> = snaps.iter().map(|f| f.view()).collect();
        let groups = find_groups(&find_blobs(&views, info.height), info.height);
        let g = match pick_ours(&groups, info.width, Side::Centre) {
            Some(i) => groups[i].clone(),
            // Receptors do not move between songs: fall back to the ones saved last time.
            None if same_display && !o.relearn => {
                eprintln!(
                    "receptors not visible; using the saved positions (--relearn to search again)"
                );
                saved_group(&prof)
            }
            None => {
                return Err(
                    "no receptor group found, or the centre rule could not pick ours".into(),
                );
            }
        };
        let g = &g;
        let guess = if same_display {
            prof.scroll
        } else {
            guess_scroll(g, info.height)
        };
        let mut layout = layout_for(g, &views, guess, stream, &dcfg);

        // Same display and receptors within 2% of the stream height (spec 03, side detection step 1).
        let same_layout = same_display
            && prof
                .receptors
                .iter()
                .zip(&layout.receptors)
                .all(|(a, b)| (a[0] - b[0]).hypot(a[1] - b[1]) < 0.02 * info.height as f32);
        // shortcut: a skin change needs --relearn; auto re-learn on a <50% colour match (spec 03) comes with the app.
        let cached = prof.colours.clone().filter(|_| same_layout && !o.relearn);
        eprintln!(
            "receptors x {:?} y {:.1}, size {:.0}, scroll {:?}",
            layout.receptors.map(|r| r[0].round()),
            layout.receptors[0][1],
            layout.size,
            layout.scroll
        );

        let (learner, colours) = if let Some(c) = cached {
            prof.layout = Some(layout.clone());
            save_profile(prof_path.as_deref(), &prof);
            eprintln!("using saved colours from the profile (--relearn to learn them again)");
            (bg_learner(src, &layout, &dcfg, o.learn_s / 2.0)?, c)
        } else {
            eprintln!("learning colours...");
            let learner = match learn(src, &layout, &dcfg, o.learn_s / 2.0) {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("learning failed ({e}); trying the other scroll direction");
                    layout.scroll = if guess == Scroll::Up {
                        Scroll::Down
                    } else {
                        Scroll::Up
                    };
                    learn(src, &layout, &dcfg, o.learn_s / 2.0)?
                }
            };
            let c = learner.finish()?;
            prof.monitor = info.monitor.clone();
            prof.stream_size = stream;
            prof.refresh_hz = info.refresh_hz;
            prof.scale = info.scale;
            prof.scroll = layout.scroll;
            prof.receptors = layout.receptors;
            prof.radius_px = layout.size / 2.0;
            prof.colours = Some(c.clone());
            prof.layout = Some(layout.clone());
            save_profile(prof_path.as_deref(), &prof);
            (learner, c)
        };
        (layout, learner, colours)
    };
    eprintln!(
        "colours (trail {}), head lengths {:?}",
        if colours.trail.is_some() {
            "yes"
        } else {
            "not yet: holds are tapped until 3 holds have been seen"
        },
        colours.head_len
    );
    let mut learning = colours.trail.is_none();
    let mut det = Detector::new(layout.clone(), dcfg, colours);
    for (l, bg) in learner.bg.iter().enumerate() {
        det.set_background(l, bg);
    }
    // Songs can start without holds (or have none): play now, learn the trail colour on a side
    // thread from every 8th frame, and hand it to the detector when found.
    let (frame_tx, frame_rx) = mpsc::sync_channel::<OwnedFrame>(2);
    let (trail_tx, trail_rx) = mpsc::channel::<Colours>();
    if learning {
        let mut learner = learner;
        std::thread::Builder::new()
            .name("trail-learner".into())
            .spawn(move || {
                let mut n = 0u32;
                while let Ok(f) = frame_rx.recv() {
                    learner.feed(&f.view());
                    n += 1;
                    if !n.is_multiple_of(20) {
                        continue;
                    }
                    if let Ok(c) = learner.finish()
                        && c.trail.is_some()
                    {
                        if let Some(pc) = prof.colours.as_mut() {
                            pc.trail = c.trail;
                            pc.trail_tol = c.trail_tol;
                        }
                        save_profile(prof_path.as_deref(), &prof);
                        let _ = trail_tx.send(c);
                        return;
                    }
                }
            })?;
    }
    let plan = PlanConfig::from_config(&cfg, o.offset_ms);
    let max_hold = plan.max_hold_ms;
    let mut eng = Engine::new(det, plan);

    // Focus guard (S3): fail open with a warning unless the config requires it.
    let focus = Arc::new(AtomicBool::new(true));
    let stop = Arc::new(AtomicBool::new(false));
    let _probe = start_focus(&cfg, focus.clone(), stop.clone())?;

    let mut sink: Box<dyn KeySink> = if o.live {
        live_sink(key_names(cfg.keys.set, &cfg.keys.custom))?
    } else {
        Box::new(NullSink)
    };
    eprintln!(
        "playing for {:.0} s, {} ({}), offset {:+.1} ms",
        o.seconds,
        if o.live {
            "LIVE: sending keys"
        } else {
            "dry run: no keys sent"
        },
        sink.name(),
        o.offset_ms
    );

    let (tx, rx) = mpsc::sync_channel::<Cmd>(256);
    let (etx, erx) = mpsc::channel::<SchedEvent>();
    let (f2, s2) = (focus.clone(), stop.clone());
    let sched = std::thread::Builder::new()
        .name("scheduler".into())
        .spawn(move || {
            eprintln!("scheduler thread: {}", make_realtime());
            let clock = RealClock;
            let mut s = Scheduler::new(max_hold);
            let mut ev = Vec::new();
            let mut last_dog = clock.now();
            while !s2.load(Ordering::SeqCst) {
                while let Ok(c) = rx.try_recv() {
                    s.submit(c);
                }
                let now = clock.now();
                match s.next_due() {
                    Some(t) if t.ms_since(now) < 2.0 => {
                        clock.sleep_until(t);
                        s.fire_due(
                            clock.now(),
                            f2.load(Ordering::SeqCst),
                            sink.as_mut(),
                            &mut ev,
                        );
                    }
                    due => {
                        let wait = due.map_or(50.0, |t| t.ms_since(now) - 1.5).clamp(0.1, 50.0);
                        if let Ok(c) = rx.recv_timeout(Duration::from_secs_f64(wait / 1000.0)) {
                            s.submit(c);
                        }
                    }
                }
                let now = clock.now();
                if now.ms_since(last_dog) >= 50.0 {
                    last_dog = now;
                    s.watchdog(now, sink.as_mut(), &mut ev);
                }
                if !f2.load(Ordering::SeqCst) {
                    // Focus lost: release everything at once (spec 05).
                    for l in 0..4 {
                        if s.is_down(l) {
                            s.release_all(sink.as_mut());
                            break;
                        }
                    }
                }
                for e in ev.drain(..) {
                    let _ = etx.send(e);
                }
            }
            s.release_all(sink.as_mut());
        })?;

    println!(
        "game focused requirement: {}",
        if cfg.safety.require_focus_guard {
            "on"
        } else {
            "off (fail open)"
        }
    );
    eprintln!("scan thread: {}", make_realtime());
    let clock = RealClock;
    let t_end = Instant::now() + Duration::from_secs_f32(o.seconds);
    let (mut scan_us, mut late_us) = (Vec::new(), Vec::new());
    let (mut pressed, mut suppressed, mut watchdog, mut frames) = (0u32, 0u32, 0u32, 0u32);
    let mut cmds = Vec::with_capacity(32);
    let mut tick = Instant::now();
    while Instant::now() < t_end {
        let Some(f) = src.next_frame(Duration::from_millis(200))? else {
            continue;
        };
        let now = clock.now();
        cmds.clear();
        let t = Instant::now();
        eng.on_frame(&f, now, &mut cmds);
        if learning {
            if let Ok(c) = trail_rx.try_recv() {
                eng.det.colours.trail = c.trail;
                eng.det.colours.trail_tol = c.trail_tol;
                learning = false;
                eprintln!("trail colour learned: holds are now held");
            } else if frames.is_multiple_of(8) {
                // shortcut: copies a frame on the scan thread (allocates) until the trail is learned; the app should use a preallocated slot.
                let _ = frame_tx.try_send(f.to_owned());
            }
        }
        scan_us.push(t.elapsed().as_secs_f64() * 1e6);
        frames += 1;
        for &c in &cmds {
            if tx.try_send(c).is_err() {
                eprintln!("scheduler queue full: dropped a command");
            }
        }
        for e in erx.try_iter() {
            match e {
                SchedEvent::Pressed {
                    planned, actual, ..
                } => {
                    pressed += 1;
                    late_us.push(actual.ms_since(planned) * 1000.0);
                }
                SchedEvent::Suppressed { .. } => suppressed += 1,
                SchedEvent::WatchdogRelease { .. } => watchdog += 1,
                SchedEvent::SinkError => eprintln!("key sink error"),
                SchedEvent::Released { .. } => {}
            }
        }
        if tick.elapsed() >= Duration::from_secs(1) {
            tick = Instant::now();
            eprintln!(
                "{frames:4} fps | scan p99 {:4.0} us | v {:>6} px/ms | presses {pressed:4} | late-found missed {} | suppressed {suppressed} | focus {}",
                pct(&mut scan_us.clone(), 99.0),
                eng.tracker
                    .velocity()
                    .map_or("-".into(), |v| format!("{:.2}", v.abs())),
                eng.planner.missed_by_engine,
                if focus.load(Ordering::SeqCst) {
                    "game"
                } else {
                    "away"
                },
            );
            frames = 0;
        }
    }
    stop.store(true, Ordering::SeqCst);
    let _ = sched.join();
    println!(
        "done: {pressed} presses, {} missed by engine, {suppressed} suppressed (focus), {watchdog} watchdog releases",
        eng.planner.missed_by_engine
    );
    if !late_us.is_empty() {
        println!(
            "scheduler lateness us p50 {:.0} p99 {:.0}; scan us p50 {:.0} p99 {:.0}",
            pct(&mut late_us, 50.0),
            pct(&mut late_us, 99.0),
            pct(&mut scan_us, 50.0),
            pct(&mut scan_us, 99.0)
        );
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn live_sink(keys: [&str; 4]) -> Result<Box<dyn KeySink>, Box<dyn std::error::Error>> {
    Ok(Box::new(arrowhead_input::uinput::UinputSink::new(keys)?))
}

#[cfg(not(target_os = "linux"))]
fn live_sink(_: [&str; 4]) -> Result<Box<dyn KeySink>, Box<dyn std::error::Error>> {
    Err("live input on this OS arrives in v0.2".into())
}

/// Polls the focus probe at 10 Hz into `focus`. Returns the probe thread's guard.
#[cfg(target_os = "linux")]
fn start_focus(
    cfg: &Config,
    focus: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
) -> Result<Option<std::thread::JoinHandle<()>>, Box<dyn std::error::Error>> {
    use arrowhead_core::input::FocusProbe;
    match arrowhead_input::kwin::KwinProbe::start(cfg.safety.game_window_classes.clone()) {
        Ok(mut probe) => Ok(Some(std::thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                // No report yet counts as focused (fail open); a report decides.
                focus.store(probe.game_focused().unwrap_or(true), Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(100));
            }
        }))),
        Err(e) if cfg.safety.require_focus_guard => {
            Err(format!("{e} (safety.require_focus_guard is on)").into())
        }
        Err(e) => {
            eprintln!("WARNING: focus guard off ({e}); keys go to whatever window is focused");
            Ok(None)
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn start_focus(
    _: &Config,
    _: Arc<AtomicBool>,
    _: Arc<AtomicBool>,
) -> Result<Option<std::thread::JoinHandle<()>>, Box<dyn std::error::Error>> {
    eprintln!("WARNING: focus guard off (no probe on this OS yet)");
    Ok(None)
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
