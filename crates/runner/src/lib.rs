//! The live engine run shared by the CLI and the app (spec 04, 05, 07): calibrate on the live
//! screen, then play, reporting everything as `EngineEvent`s.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, SyncSender};
use std::time::{Duration, Instant};

use arrowhead_core::config::{Config, Profile, Scroll, Side, config_dir, load_toml, save_toml};
use arrowhead_core::detect::calib::{
    Blob, ColourLearner, Group, Presence, find_blobs, find_groups, guess_scroll, layout_for,
    pick_ours,
};
use arrowhead_core::detect::{Colours, DetectConfig, Detector, Layout};
use arrowhead_core::engine::Engine;
use arrowhead_core::event::{Dot, Dots, EngineEvent, Level, Pipeline, SuppressReason, ms};
use arrowhead_core::frame::{FrameSource, FrameView, OwnedFrame};
use arrowhead_core::input::{Hotkey, KeySink, NullSink, key_names};
use arrowhead_core::plan::{Cmd, PlanConfig};
use arrowhead_core::runtime::platform::{EXIT, make_realtime};
use arrowhead_core::sched::{SchedEvent, Scheduler};
use arrowhead_core::session::{Event as SEv, Session};
use arrowhead_core::time::{Clock, RealClock};

mod flight;
mod hud;
pub mod latency;
mod ocrteach;
mod ocrwork;
pub mod platform;

pub use platform::{live_sink, start_focus, start_hotkeys};

pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub type Res<T> = Result<T, Error>;
pub type Tx = SyncSender<EngineEvent>;

/// Never blocks: a full bus drops the event (the UI only shows the latest anyway).
pub fn emit(tx: &Tx, e: EngineEvent) {
    let _ = tx.try_send(e);
}

pub fn log(tx: &Tx, level: Level, text: impl Into<String>) {
    emit(
        tx,
        EngineEvent::Log {
            level,
            text: text.into(),
        },
    );
}

/// Shared switches, flipped by the UI, the CLI and hotkeys while the engine runs.
#[derive(Default)]
pub struct Control {
    pub stop: AtomicBool,
    pub paused: AtomicBool,
    pub dry_run: AtomicBool,
}

pub struct Opts {
    /// Open the real key sink (uinput). Without it nothing can be pressed, whatever `dry_run` says.
    pub live: bool,
    /// None: until stopped.
    pub seconds: Option<f32>,
    /// None: the profile's game bias.
    pub offset_ms: Option<f64>,
    pub learn_s: f32,
    pub relearn: bool,
}

pub fn load_config() -> Res<Config> {
    Ok(config_dir()
        .map(|d| load_toml(&d.join("config.toml")))
        .transpose()?
        .unwrap_or_default())
}

pub fn profile_path(cfg: &Config) -> Option<PathBuf> {
    config_dir().map(|d| d.join("profiles").join(format!("{}.toml", cfg.profile)))
}

fn next(src: &mut dyn FrameSource) -> Res<OwnedFrame> {
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
) -> Res<ColourLearner> {
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

fn feed(src: &mut dyn FrameSource, learner: &mut ColourLearner, secs: f32) -> Res<()> {
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
) -> Res<ColourLearner> {
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

fn save_profile(tx: &Tx, path: Option<&std::path::Path>, prof: &Profile) {
    if let Some(p) = path
        && let Err(e) = save_toml(p, prof)
    {
        log(tx, Level::Warn, format!("could not save the profile: {e}"));
    }
}

/// Result of calibration: the layout (with the profile's nudge applied), the colours and the
/// learner (its background, and trail learning if the trail is not known yet).
pub struct Calibrated {
    pub cfg: Config,
    pub prof: Profile,
    pub prof_path: Option<PathBuf>,
    /// Detected layout, before the nudge.
    pub raw_layout: Layout,
    pub layout: Layout,
    pub colours: Colours,
    pub learner: ColourLearner,
    /// A full-stream frame from calibration, for the Setup snapshot.
    pub snapshot: Option<OwnedFrame>,
}

pub fn calibrate(
    src: &mut dyn FrameSource,
    learn_s: f32,
    relearn: bool,
    tx: &Tx,
) -> Res<Calibrated> {
    let cfg = load_config()?;
    let info = src.info();
    let dcfg = DetectConfig::default();
    log(
        tx,
        Level::Info,
        format!(
            "stream {}x{} @ {:.0} Hz ({}). Calibrating: have a song playing, keep the game visible.",
            info.width, info.height, info.refresh_hz, info.backend
        ),
    );
    let stream = [info.width, info.height];
    let prof_path = profile_path(&cfg);
    let mut prof: Profile = prof_path
        .as_deref()
        .map(load_toml)
        .transpose()?
        .unwrap_or_default();
    let same_display = prof.matches(&info.monitor, stream, info.scale) && prof.radius_px > 0.0;
    // Receptors and colours do not change between songs: with a saved layout only the stage
    // background (1.5 s; the detector keeps adapting it) is learned before play.
    let saved = match (&prof.layout, &prof.colours) {
        (Some(l), Some(c)) if same_display && !relearn => Some((l.clone(), c.clone())),
        _ => None,
    };
    let mut snapshot = None;
    let (raw_layout, learner, colours) = if let Some((layout, c)) = saved {
        log(
            tx,
            Level::Info,
            "using the saved layout and colours (re-run Calibrate if the game window moved or the skin changed)",
        );
        let l = prof.nudge.apply(&layout);
        (layout, bg_learner(src, &l, &dcfg, 1.5)?, c)
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
            None if same_display && !relearn => {
                log(
                    tx,
                    Level::Warn,
                    "receptors not visible; using the saved positions",
                );
                saved_group(&prof)
            }
            None => {
                return Err("no receptor group found, or the centre rule could not pick ours. Start a song and keep the game visible".into());
            }
        };
        let guess = if same_display {
            prof.scroll
        } else {
            guess_scroll(&g, info.height)
        };
        let mut layout = layout_for(&g, &views, guess, stream, &dcfg);
        drop(views);
        snapshot = snaps.pop();

        // Same display and receptors within 2% of the stream height (spec 03, side detection step 1).
        let same_layout = same_display
            && prof
                .receptors
                .iter()
                .zip(&layout.receptors)
                .all(|(a, b)| (a[0] - b[0]).hypot(a[1] - b[1]) < 0.02 * info.height as f32);
        // shortcut: a skin change needs a relearn; auto re-learn on a <50% colour match (spec 03) is not built.
        let cached = prof.colours.clone().filter(|_| same_layout && !relearn);
        log(
            tx,
            Level::Info,
            format!(
                "receptors x {:?} y {:.1}, size {:.0}, scroll {:?}",
                layout.receptors.map(|r| r[0].round()),
                layout.receptors[0][1],
                layout.size,
                layout.scroll
            ),
        );
        if !same_layout {
            // A new layout invalidates the old hand correction.
            prof.nudge = Default::default();
        }
        let (learner, colours) = if let Some(c) = cached {
            prof.layout = Some(layout.clone());
            save_profile(tx, prof_path.as_deref(), &prof);
            log(tx, Level::Info, "using saved colours from the profile");
            (
                bg_learner(src, &prof.nudge.apply(&layout), &dcfg, learn_s / 2.0)?,
                c,
            )
        } else {
            log(tx, Level::Info, "learning colours...");
            let learner = match learn(src, &layout, &dcfg, learn_s / 2.0) {
                Ok(l) => l,
                Err(e) => {
                    log(
                        tx,
                        Level::Warn,
                        format!("learning failed ({e}); trying the other scroll direction"),
                    );
                    layout.scroll = if guess == Scroll::Up {
                        Scroll::Down
                    } else {
                        Scroll::Up
                    };
                    learn(src, &layout, &dcfg, learn_s / 2.0)?
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
            save_profile(tx, prof_path.as_deref(), &prof);
            (learner, c)
        };
        (layout, learner, colours)
    };
    log(
        tx,
        Level::Info,
        format!(
            "colours learned (trail {})",
            if colours.trail.is_some() {
                "yes"
            } else {
                "not yet: holds are tapped until 3 holds have been seen"
            }
        ),
    );
    let layout = prof.nudge.apply(&raw_layout);
    Ok(Calibrated {
        cfg,
        prof,
        prof_path,
        raw_layout,
        layout,
        colours,
        learner,
        snapshot,
    })
}

/// Fixed ring of samples with percentiles, so the scan thread never allocates for its stats.
struct Samples {
    v: [f32; 512],
    n: usize,
}

impl Samples {
    fn new() -> Self {
        Self {
            v: [0.0; 512],
            n: 0,
        }
    }

    fn push(&mut self, x: f32) {
        self.v[self.n % 512] = x;
        self.n += 1;
    }

    fn pct(&self, p: f32) -> f32 {
        let n = self.n.min(512);
        if n == 0 {
            return 0.0;
        }
        let mut c = self.v;
        let s = &mut c[..n];
        let k = ((n - 1) as f32 * p / 100.0).round() as usize;
        *s.select_nth_unstable_by(k, |a, b| a.total_cmp(b)).1
    }

    fn clear(&mut self) {
        self.n = 0;
    }
}

#[derive(Debug, Default)]
pub struct Summary {
    pub presses: u32,
    pub missed_by_engine: u32,
    pub suppressed: u32,
    pub watchdog: u32,
}

/// Plays until `ctl.stop`, `EXIT`, a panic hotkey or `o.seconds`. Calibrates first.
pub fn run(src: &mut dyn FrameSource, o: Opts, ctl: Arc<Control>, tx: Tx) -> Res<Summary> {
    let state = |tx: &Tx, s, reason| {
        emit(
            tx,
            EngineEvent::State {
                state: s,
                dry_run: ctl.dry_run.load(Ordering::SeqCst),
                reason,
            },
        );
    };
    let mut session = Session::default();
    if let Some(s) = session.on(SEv::Started) {
        state(&tx, s, "calibrating");
    }
    let Calibrated {
        cfg,
        prof,
        prof_path,
        layout,
        colours,
        learner,
        ..
    } = calibrate(src, o.learn_s, o.relearn, &tx)?;
    emit(
        &tx,
        EngineEvent::Layout {
            receptors: layout.receptors,
            size: layout.size,
            scroll: layout.scroll,
            stream: layout.stream,
            profile: cfg.profile.clone(),
        },
    );
    if let Some(s) = session.on(SEv::LayoutReady) {
        state(&tx, s, "layout ready");
    }
    // Not -input_ms: the game hides most of its own render delay, so the measured 19 ms made
    // presses early; a hand-tuned -5.2 ms played best (2026-11-22). Readback will tune it in v0.2.
    let offset_ms = o.offset_ms.unwrap_or(prof.latency.game_bias_ms as f64);
    let mut learning = colours.trail.is_none();
    let dcfg = DetectConfig::default();
    let dcfg_hud = dcfg.clone();
    let prof_path_hud = prof_path.clone();
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
        let mut prof = prof.clone();
        let tx = tx.clone();
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
                        // The HUD thread saves the profile too: start from what is on disk.
                        if let Some(p) = prof_path.as_deref()
                            && let Ok(disk) = load_toml::<Profile>(p)
                        {
                            prof = disk;
                        }
                        if let Some(pc) = prof.colours.as_mut() {
                            pc.trail = c.trail;
                            pc.trail_tol = c.trail_tol;
                        }
                        save_profile(&tx, prof_path.as_deref(), &prof);
                        let _ = trail_tx.send(c);
                        return;
                    }
                }
            })?;
    }
    let plan = PlanConfig::from_config(&cfg, offset_ms);
    let max_hold = plan.max_hold_ms;
    let mut eng = Engine::new(det, plan);
    // Player style from config.toml (spec 06); Max or an unknown name leaves the planner unchanged.
    let style = arrowhead_core::style::find_style(config_dir().as_deref(), &cfg.style);
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64);
    let (style_name, style_target) = (
        style.name.clone(),
        (!style.is_max()).then_some(style.target as f32),
    );
    if !style.is_max() {
        log(
            &tx,
            Level::Info,
            format!("style: {} (target {}%)", style.name, style.target),
        );
        eng.planner
            .set_style(arrowhead_core::style::StylePlayer::new(
                style, &cfg.game, seed, 600,
            ));
    }

    // Flight recorder (spec 13 3.2, off by default): last 10 s of frames and presses, dumped
    // when the HUD sees a miss.
    let (mut flight, flight_hud) = if cfg.debug.flight_recorder {
        let i = src.info();
        let (f, h) = flight::Flight::start(
            flight::Stream {
                width: i.width,
                height: i.height,
                refresh_hz: i.refresh_hz,
                backend: i.backend.clone(),
            },
            tx.clone(),
        )?;
        log(
            &tx,
            Level::Info,
            "flight recorder on: a miss saves the last 10 s to data/flight/",
        );
        (Some(f), Some(h))
    } else {
        (None, None)
    };

    // HUD reader and readback (spec 03, 04) on their own thread; channels only.
    let (hud_tx, hud_rx) = mpsc::sync_channel::<hud::ToHud>(256);
    let (from_hud_tx, from_hud) = mpsc::channel::<hud::FromHud>();
    let r = layout.receptors;
    if !cfg.ui.hud_reader {
        log(
            &tx,
            Level::Info,
            "HUD reader off (Settings): playing on the saved offset",
        );
    }
    let hud_thread = hud::spawn(
        src.aux().filter(|_| cfg.ui.hud_reader),
        hud::HudSetup {
            profile: cfg.profile.clone(),
            prof_path: prof_path_hud,
            hud: prof.hud.clone(),
            stream: layout.stream,
            receptors_x: r.map(|p| p[0]),
            pitch: (r[3][0] - r[0][0]).abs() / 3.0,
            max_sad: dcfg_hud.glyph_max_sad,
            offset_ms: offset_ms as f32,
            readback_sign: f32::from(cfg.game.readback_sign),
            stats_mean_sign: f32::from(cfg.game.stats_mean_sign),
            windows: cfg.game.windows_ms.clone(),
            style: style_name,
            target: style_target,
            seed,
            ocr_model: cfg.ui.ocr_model.clone(),
            seat: cfg.game.seat,
            match_friend: cfg.match_friend.clone(),
            flight: flight_hud,
        },
        hud_rx,
        from_hud_tx,
        tx.clone(),
    )?;

    // Focus guard (S3): fail open with a warning unless the config requires it.
    let focus = Arc::new(AtomicBool::new(true));
    let sched_stop = Arc::new(AtomicBool::new(false));
    let (hk_tx, hk_rx) = mpsc::channel::<Hotkey>();
    let (_hotkeys, hk_bound) = start_hotkeys(&cfg, hk_tx, &tx);
    let has_probe = start_focus(&cfg, focus.clone(), sched_stop.clone(), &tx)?;
    emit(
        &tx,
        EngineEvent::Focus {
            focused: has_probe.then_some(true),
        },
    );

    let mut sink: Box<dyn KeySink> = if o.live {
        live_sink(key_names(cfg.keys.set, &cfg.keys.custom))?
    } else {
        ctl.dry_run.store(true, Ordering::SeqCst);
        Box::new(NullSink)
    };
    log(
        &tx,
        Level::Info,
        format!(
            "playing{}, {} ({}), offset {:+.1} ms, focus guard {}",
            o.seconds
                .map_or(String::new(), |s| format!(" for {s:.0} s")),
            if ctl.dry_run.load(Ordering::SeqCst) {
                "dry run: no keys sent"
            } else {
                "LIVE: sending keys"
            },
            sink.name(),
            offset_ms,
            if cfg.safety.require_focus_guard {
                "required"
            } else {
                "fail open"
            }
        ),
    );

    let (cmd_tx, cmd_rx) = mpsc::sync_channel::<Cmd>(256);
    let (etx, erx) = mpsc::sync_channel::<SchedEvent>(1024);
    let (f2, s2, c2) = (focus.clone(), sched_stop.clone(), ctl.clone());
    let tx2 = tx.clone();
    let sched = std::thread::Builder::new()
        .name("scheduler".into())
        .spawn(move || {
            log(
                &tx2,
                Level::Info,
                format!("scheduler thread: {}", make_realtime()),
            );
            let clock = RealClock;
            let mut s = Scheduler::new(max_hold);
            let mut ev = Vec::with_capacity(64);
            let mut last_dog = clock.now();
            // EXIT also covers a panic on the scan thread (S2).
            while !s2.load(Ordering::SeqCst) && !EXIT.load(Ordering::SeqCst) {
                let gate = f2.load(Ordering::SeqCst)
                    && !c2.paused.load(Ordering::SeqCst)
                    && !c2.dry_run.load(Ordering::SeqCst);
                while let Ok(c) = cmd_rx.try_recv() {
                    s.submit(c);
                }
                let now = clock.now();
                match s.next_due() {
                    Some(t) if t.ms_since(now) < 2.0 => {
                        clock.sleep_until(t);
                        s.fire_due(clock.now(), gate, sink.as_mut(), &mut ev);
                    }
                    due => {
                        let wait = due.map_or(50.0, |t| t.ms_since(now) - 1.5).clamp(0.1, 50.0);
                        if let Ok(c) = cmd_rx.recv_timeout(Duration::from_secs_f64(wait / 1000.0)) {
                            s.submit(c);
                        }
                    }
                }
                let now = clock.now();
                if now.ms_since(last_dog) >= 50.0 {
                    last_dog = now;
                    s.watchdog(now, sink.as_mut(), &mut ev);
                }
                if !gate && (0..4).any(|l| s.is_down(l)) {
                    // Focus lost, paused or dry run: release everything at once (spec 05).
                    s.release_all(sink.as_mut());
                }
                for e in ev.drain(..) {
                    let _ = etx.try_send(e);
                }
            }
            s.release_all(sink.as_mut());
        })?;

    log(
        &tx,
        Level::Info,
        format!("scan thread: {}", make_realtime()),
    );
    let clock = RealClock;
    let t_end = o
        .seconds
        .map(|s| Instant::now() + Duration::from_secs_f32(s));
    let (mut scan_us, mut late_us, mut interval, mut age) = (
        Samples::new(),
        Samples::new(),
        Samples::new(),
        Samples::new(),
    );
    let mut sum = Summary::default();
    let mut frames = 0u32;
    let mut cmds = Vec::with_capacity(32);
    let mut tick = Instant::now();
    let mut last_notes = Instant::now();
    let mut last_pts = None;
    let mut last_head = Instant::now();
    // Receptor presence (spec 03, session): templates from frames taken once notes first
    // approach (the receptors are surely on screen then); a song ends when they are gone
    // for 1.5 s (results screen, menu), so breaks inside a song do not split it.
    let mut presence: Option<Presence> = None;
    let mut pres_frames: Vec<OwnedFrame> = Vec::new();
    let mut absent_since: Option<Instant> = None;
    let (mut was_paused, mut was_focused, mut was_dry) =
        (false, true, ctl.dry_run.load(Ordering::SeqCst));
    let mut err = None;
    while !ctl.stop.load(Ordering::SeqCst)
        && !EXIT.load(Ordering::SeqCst)
        && t_end.is_none_or(|t| Instant::now() < t)
    {
        if let Ok(r) = hk_bound.try_recv() {
            let (bound, error) = match r {
                Ok(list) => (Some(list), None),
                Err(e) => (None, Some(e)),
            };
            emit(&tx, EngineEvent::Hotkeys { bound, error });
        }
        for h in hk_rx.try_iter() {
            match h {
                Hotkey::Toggle => {
                    ctl.paused.fetch_xor(true, Ordering::SeqCst);
                }
                Hotkey::Panic => {
                    log(&tx, Level::Warn, "PANIC: releasing all keys and stopping");
                    ctl.stop.store(true, Ordering::SeqCst);
                }
                Hotkey::NudgeUp | Hotkey::NudgeDown => {
                    let d = if h == Hotkey::NudgeUp { 1.0 } else { -1.0 };
                    // Styled: +/- 0.5% target; Max: +/- 1 ms offset (spec 05).
                    if let Some(p) = eng.planner.style_mut() {
                        p.style.target = (p.style.target + 0.5 * d).clamp(70.0, 100.0);
                        log(&tx, Level::Info, format!("target {:.1}%", p.style.target));
                    } else {
                        eng.planner.cfg.offset_ms += d;
                        log(
                            &tx,
                            Level::Info,
                            format!("offset {:+.1} ms", eng.planner.cfg.offset_ms),
                        );
                    }
                }
                Hotkey::CycleStyle => {
                    // Next style in the list; it takes over from the next note.
                    let list = config_dir()
                        .map_or_else(arrowhead_core::style::Style::builtins, |d| {
                            arrowhead_core::style::list_styles(&d)
                        });
                    let cur = eng.planner.style().map_or("Max", |p| p.style.name.as_str());
                    let i = list
                        .iter()
                        .position(|st| st.name == cur)
                        .map_or(0, |i| i + 1);
                    let next = list[i % list.len()].clone();
                    log(&tx, Level::Info, format!("style: {}", next.name));
                    let mut c = load_config().unwrap_or_default();
                    c.style = next.name.clone();
                    if let Some(d) = config_dir() {
                        let _ = save_toml(&d.join("config.toml"), &c);
                    }
                    eng.planner
                        .set_style(arrowhead_core::style::StylePlayer::new(
                            next, &cfg.game, seed, 600,
                        ));
                }
            }
        }
        // Pause / focus / dry-run changes drive the session state.
        let (p, f, d) = (
            ctl.paused.load(Ordering::SeqCst),
            focus.load(Ordering::SeqCst),
            ctl.dry_run.load(Ordering::SeqCst),
        );
        let mut changed = None;
        if p != was_paused {
            was_paused = p;
            changed = session.on(SEv::TogglePause).or(changed);
        }
        if f != was_focused {
            was_focused = f;
            emit(&tx, EngineEvent::Focus { focused: Some(f) });
            changed = session
                .on(if f { SEv::FocusGained } else { SEv::FocusLost })
                .or(changed);
        }
        if let Some(s) = changed {
            state(
                &tx,
                s,
                if p {
                    "paused"
                } else if !f {
                    "game not focused"
                } else {
                    "resumed"
                },
            );
        } else if d != was_dry {
            state(
                &tx,
                session.state(),
                if d { "dry run on" } else { "dry run off" },
            );
        }
        was_dry = d;

        for m in from_hud.try_iter() {
            match m {
                hud::FromHud::Offset(o) => eng.planner.cfg.offset_ms = f64::from(o),
                hud::FromHud::Accuracy(a) => eng.planner.feed_accuracy(Some(f64::from(a))),
                hud::FromHud::Corrections(l, c) => eng.planner.set_corrections(l, c),
                hud::FromHud::Target(t) => {
                    if let Some(p) = eng.planner.style_mut() {
                        p.style.target = t.clamp(70.0, 100.0);
                    }
                }
            }
        }

        let f = match src.next_frame(Duration::from_millis(200)) {
            Ok(Some(f)) => f,
            Ok(None) => continue,
            Err(e) => {
                err = Some(e);
                break;
            }
        };
        let now = clock.now();
        cmds.clear();
        let t = Instant::now();
        eng.on_frame(&f, now, &mut cmds);
        if let Some(fl) = flight.as_mut() {
            fl.frame(&f);
        }
        if learning {
            if let Ok(c) = trail_rx.try_recv() {
                eng.det.colours.trail = c.trail;
                eng.det.colours.trail_tol = c.trail_tol;
                learning = false;
                log(&tx, Level::Info, "trail colour learned: holds are now held");
            } else if frames.is_multiple_of(8) {
                // shortcut: copies a frame on the scan thread (allocates) until the trail is learned; a preallocated slot would avoid it.
                let _ = frame_tx.try_send(f.to_owned());
            }
        }
        scan_us.push(t.elapsed().as_secs_f32() * 1e6);
        age.push(now.ms_since(f.pts) as f32);
        if let Some(p) = last_pts.replace(f.pts) {
            interval.push(f.pts.ms_since(p) as f32);
        }
        frames += 1;
        for &c in &cmds {
            if cmd_tx.try_send(c).is_err() {
                log(&tx, Level::Warn, "scheduler queue full: dropped a command");
            } else if let Cmd::Press {
                id,
                lane,
                at,
                release,
            } = c
            {
                if let Some(fl) = &flight {
                    fl.send(flight::Msg::Planned {
                        id,
                        lane,
                        at,
                        release,
                    });
                }
                emit(
                    &tx,
                    EngineEvent::Planned {
                        id,
                        lane,
                        at_ms: ms(at),
                        release_ms: ms(release),
                    },
                );
            }
        }
        let suppress_reason = if d {
            SuppressReason::DryRun
        } else if p {
            SuppressReason::Paused
        } else {
            SuppressReason::Focus
        };
        for e in erx.try_iter() {
            match e {
                SchedEvent::Pressed {
                    id,
                    lane,
                    planned,
                    actual,
                } => {
                    sum.presses += 1;
                    if let Some(fl) = &flight {
                        fl.send(flight::Msg::Sent {
                            id,
                            lane,
                            planned,
                            actual,
                        });
                    }
                    late_us.push((actual.ms_since(planned) * 1000.0) as f32);
                    let _ = hud_tx.try_send(hud::ToHud::Press {
                        sent: actual,
                        lane,
                        intended_ms: eng.planner.intended_err(id),
                        class: eng.planner.class_of(id),
                    });
                    emit(
                        &tx,
                        EngineEvent::Pressed {
                            id,
                            lane,
                            planned_ms: ms(planned),
                            actual_ms: ms(actual),
                        },
                    );
                }
                SchedEvent::Released {
                    id,
                    lane,
                    planned,
                    actual,
                } => {
                    emit(
                        &tx,
                        EngineEvent::Released {
                            id,
                            lane,
                            planned_ms: ms(planned),
                            actual_ms: ms(actual),
                        },
                    );
                }
                SchedEvent::Suppressed { id, lane } => {
                    sum.suppressed += 1;
                    emit(
                        &tx,
                        EngineEvent::Suppressed {
                            id,
                            lane,
                            reason: suppress_reason,
                        },
                    );
                }
                SchedEvent::WatchdogRelease { lane } => {
                    sum.watchdog += 1;
                    log(&tx, Level::Warn, format!("watchdog released lane {lane}"));
                }
                SchedEvent::SinkError => log(&tx, Level::Error, "key sink error"),
            }
        }

        // Session: a confirmed head means a song is on; receptors gone for 1.5 s mean it is over.
        if session.state() == arrowhead_core::session::State::Playing {
            match &presence {
                None if frames.is_multiple_of(16) => {
                    // shortcut: 9 frame copies on the scan thread, once per run, to build the templates.
                    pres_frames.push(f.to_owned());
                    if pres_frames.len() == 9 {
                        let views: Vec<FrameView> = pres_frames.iter().map(|f| f.view()).collect();
                        presence = Some(Presence::new(&layout, &views, &dcfg_hud));
                        pres_frames = Vec::new();
                    }
                }
                Some(p) if frames.is_multiple_of(10) => {
                    if p.check(&f) {
                        absent_since = None;
                    } else {
                        absent_since.get_or_insert_with(Instant::now);
                    }
                }
                _ => {}
            }
        }
        let receptors_gone =
            absent_since.is_some_and(|t| t.elapsed() > Duration::from_millis(1500));
        // Without templates yet, a long silence still ends the song.
        let song_over =
            receptors_gone || (presence.is_none() && last_head.elapsed() > Duration::from_secs(20));
        let tracks = eng.tracker.tracks();
        if tracks.iter().any(|t| t.confirmed && !t.done) && !receptors_gone {
            last_head = Instant::now();
            if let Some(s) = session.on(SEv::HeadApproaching) {
                state(&tx, s, "notes approaching");
                if s == arrowhead_core::session::State::Playing {
                    let _ = hud_tx.try_send(hud::ToHud::SongStart);
                }
            }
        } else if song_over && let Some(s) = session.on(SEv::SongOver) {
            absent_since = None;
            state(
                &tx,
                s,
                if receptors_gone {
                    "receptors gone: song over"
                } else {
                    "no notes for 20 s"
                },
            );
            // The HUD thread writes the history row; the session goes straight back to waiting.
            let _ = hud_tx.try_send(hud::ToHud::SongOver {
                offset_ms: eng.planner.cfg.offset_ms as f32,
                engine_missed: eng.planner.missed_by_engine,
                watchdog: sum.watchdog,
            });
            for ev in [SEv::HistoryWritten, SEv::LayoutReady] {
                if let Some(s) = session.on(ev) {
                    state(&tx, s, "waiting for the next song");
                }
            }
        }

        if last_notes.elapsed() >= Duration::from_millis(16) {
            last_notes = Instant::now();
            let mut dots = Dots::default();
            for t in tracks.iter().filter(|t| !t.done && t.n > 0) {
                dots.push(Dot {
                    id: t.id,
                    lane: t.lane,
                    y: t.last().1,
                    tail: t.tail,
                });
            }
            emit(
                &tx,
                EngineEvent::Notes {
                    t_ms: ms(now),
                    dots,
                },
            );
        }
        if tick.elapsed() >= Duration::from_secs(1) {
            let secs = tick.elapsed().as_secs_f32();
            tick = Instant::now();
            sum.missed_by_engine = eng.planner.missed_by_engine;
            emit(
                &tx,
                EngineEvent::Pipeline(Pipeline {
                    fps: frames as f32 / secs,
                    interval_p50_ms: interval.pct(50.0),
                    interval_p99_ms: interval.pct(99.0),
                    pts_age_ms: age.pct(50.0),
                    scan_us_p99: scan_us.pct(99.0),
                    late_us_p99: late_us.pct(99.0),
                    offset_ms: eng.planner.cfg.offset_ms as f32,
                    speed_px_ms: eng.tracker.velocity().map_or(0.0, |v| v.abs() as f32),
                    presses: sum.presses,
                    missed_by_engine: sum.missed_by_engine,
                    suppressed: sum.suppressed,
                    watchdog: sum.watchdog,
                }),
            );
            frames = 0;
            for s in [&mut scan_us, &mut interval, &mut age] {
                s.clear();
            }
        }
    }
    sched_stop.store(true, Ordering::SeqCst);
    let _ = sched.join();
    drop(hud_tx);
    let _ = hud_thread.join();
    sum.missed_by_engine = eng.planner.missed_by_engine;
    if let Some(e) = err {
        return Err(e.into());
    }
    if EXIT.load(Ordering::SeqCst) || ctl.stop.load(Ordering::SeqCst) {
        log(&tx, Level::Info, "stopped: all keys released");
    }
    Ok(sum)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_pct() {
        let mut s = Samples::new();
        assert_eq!(s.pct(50.0), 0.0);
        for i in 0..1000 {
            s.push(i as f32);
        }
        // Only the last 512 are kept.
        assert_eq!(s.pct(0.0), 488.0);
        assert_eq!(s.pct(100.0), 999.0);
    }
}
