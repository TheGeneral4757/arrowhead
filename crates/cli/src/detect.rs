//! `detect <clip>`: calibrate on the clip's first seconds, then print observations per frame.
//! With `--profile`, start the way a live run does instead: the profile's saved layout (with its
//! nudge) and colours, the stage background from the first 1.5 s, and (with `--plan`) the
//! config's timing, offset and style, so a mid-song clip replays without calibrating.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use arrowhead_capture::clip::{ClipHeader, ClipReader};
use arrowhead_core::config::{Profile, Scroll, Side, config_dir, load_toml};
use arrowhead_core::detect::calib::{
    ColourLearner, find_blobs, find_groups, guess_scroll, layout_for, pick_ours,
};
use arrowhead_core::detect::{Colours, DetectConfig, Detector, Layout, NoteObs, ObsKind};
use arrowhead_core::engine::Engine;
use arrowhead_core::frame::{FrameView, OwnedFrame};
use arrowhead_core::input::{KeyAction, RecordingSink};
use arrowhead_core::plan::{Cmd, PlanConfig};
use arrowhead_core::sched::Scheduler;
use arrowhead_core::style::StylePlayer;
use arrowhead_core::time::{Clock, VirtualClock};

use crate::{Res, pct};

pub struct Opts<'a> {
    pub clip: &'a Path,
    pub learn_s: f32,
    pub debug_png: Option<&'a Path>,
    pub every: usize,
    pub quiet: bool,
    /// Also run tracker + planner (dry, no keys) and report what would be pressed.
    pub plan: bool,
    /// Start from this profile's layout and colours, like a live run ("" = the config's).
    pub profile: Option<&'a str>,
    /// Fraction of frames (0-1) that repeat the previous frame's pixels with a new pts.
    pub repeat_frames: f32,
}

fn spread(reader: &mut ClipReader, n: usize, upto: usize) -> std::io::Result<Vec<OwnedFrame>> {
    (0..n)
        .map(|i| reader.read(i * (upto - 1) / (n - 1).max(1)))
        .collect()
}

fn learn(
    reader: &mut ClipReader,
    layout: &Layout,
    cfg: &DetectConfig,
    upto: usize,
) -> Result<ColourLearner, Box<dyn std::error::Error>> {
    let bg = spread(reader, 21, upto)?;
    let views: Vec<FrameView> = bg.iter().map(|f| f.view()).collect();
    let mut learner = ColourLearner::new(layout.clone(), cfg.clone(), &views);
    for i in 0..upto {
        learner.feed(&reader.read(i)?.view());
    }
    learner.finish()?;
    Ok(learner)
}

pub fn run(o: Opts) -> Res {
    let mut reader = ClipReader::open(o.clip)?;
    let h = reader.header.clone();
    let n = reader.len();
    if n < 2 {
        return Err("clip has fewer than 2 frames".into());
    }
    // Learning window by recorded time, not nominal rate (frames may be dropped).
    let t0 = reader.read(0)?.pts;
    let (mut lo, mut hi) = (1, n);
    while lo < hi {
        let mid = (lo + hi) / 2;
        if reader.read(mid)?.pts.ms_since(t0) < o.learn_s as f64 * 1000.0 {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    let upto = lo.clamp(2, n);
    let cfg = DetectConfig::default();
    let (layout, colours, bg, plan_cfg) = match o.profile {
        Some(name) => from_profile(&mut reader, name, n)?,
        None => calibrate(&mut reader, &h, upto, &cfg)?,
    };
    println!("trail     {:?} tol {}", colours.trail, colours.trail_tol);
    let region = layout.region(&cfg);
    println!(
        "region    {},{},{},{}",
        region.x, region.y, region.w, region.h
    );
    if region.intersect(&h.region) != Some(region) {
        eprintln!(
            "warning: the playfield region is not inside the clip's region {:?}",
            h.region
        );
    }

    let mut det = Detector::new(layout.clone(), cfg.clone(), colours.clone());
    let mut planner_det = Detector::new(layout.clone(), cfg, colours);
    for (l, bg) in bg.iter().enumerate() {
        det.set_background(l, bg);
        planner_det.set_background(l, bg);
    }
    let mut eng = o.plan.then(|| {
        let (plan, style) = plan_cfg;
        let mut e = Engine::new(planner_det, plan);
        if let Some(s) = style {
            e.planner.set_style(s);
        }
        e
    });

    let (mut cmds, mut presses, mut holds) = (Vec::new(), [0u32; 4], 0u32);
    // The real scheduler on a virtual clock: what the keys would have done.
    let clock = Arc::new(VirtualClock::default());
    let mut sink = RecordingSink::new(clock.clone());
    let mut sched = Scheduler::new(PlanConfig::default().max_hold_ms);
    let mut sev = Vec::new();
    let mut fired = 0usize;
    let (mut buf, mut scratch, mut rng) = (Vec::new(), Vec::new(), 1u64);
    let mut obs: Vec<NoteObs> = Vec::with_capacity(64);
    let mut prev: Vec<NoteObs> = Vec::with_capacity(64);
    let mut times = Vec::with_capacity(n);
    let (mut heads, mut trails, mut votes) = (0usize, 0usize, [0i64; 2]);
    let out = std::io::stdout();
    let mut out = BufWriter::new(out.lock());
    for i in 0..n {
        let (pts, seq) = reader.read_into(i, &mut scratch)?;
        // --repeat-frames: this frame shows the last one's pixels again, as a game that drew
        // no new frame does (spec 13, 4.2)
        rng = rng
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        if i == 0 || ((rng >> 33) as f32 / (1u64 << 31) as f32) >= o.repeat_frames {
            std::mem::swap(&mut buf, &mut scratch);
        }
        let f = FrameView {
            seq,
            pts,
            region: h.region,
            stride: h.region.w as usize * 4,
            data: &buf,
        };
        let t0 = Instant::now();
        det.detect(&f, &mut obs);
        if let Some(eng) = eng.as_mut() {
            cmds.clear();
            // As if the frame reached us 3 ms after it was shown.
            let now = pts.add_ms(3.0);
            while let Some(t) = sched.next_due().filter(|&t| t <= now) {
                clock.sleep_until(t);
                sched.fire_due(clock.now(), true, &mut sink, &mut sev);
            }
            clock.set(now);
            for (t, a) in &sink.log[fired..] {
                if !o.quiet {
                    writeln!(out, "key {a:?} at {:.1} ms", t.ms())?;
                }
            }
            fired = sink.log.len();
            eng.on_frame(&f, now, &mut cmds);
            for c in &cmds {
                match *c {
                    Cmd::Press {
                        id,
                        lane,
                        at,
                        release,
                    } => {
                        presses[lane as usize] += 1;
                        if release.ms_since(at) > 60.0 {
                            holds += 1;
                        }
                        if !o.quiet {
                            writeln!(
                                out,
                                "press lane {lane} at {:.1} ms hold {:.0} ms id {id}",
                                at.ms(),
                                release.ms_since(at)
                            )?;
                        }
                    }
                    Cmd::MoveRelease { id, release } if !o.quiet => {
                        writeln!(out, "release id {id} at {:.1} ms", release.ms())?;
                    }
                    Cmd::MoveRelease { .. } => {}
                }
                sched.submit(*c);
            }
        }
        times.push(t0.elapsed().as_secs_f64() * 1e6);
        // Scroll vote: heads move toward smaller y in upscroll.
        for o in obs.iter().filter(|o| o.kind == ObsKind::Head) {
            let near = prev
                .iter()
                .filter(|p| p.kind == ObsKind::Head && p.lane == o.lane)
                .map(|p| o.y - p.y)
                .filter(|d| d.abs() > 0.5 && d.abs() < layout.size / 2.0)
                .min_by(|a, b| a.abs().total_cmp(&b.abs()));
            if let Some(d) = near {
                votes[(d > 0.0) as usize] += 1;
            }
        }
        heads += obs.iter().filter(|o| o.kind == ObsKind::Head).count();
        trails += obs.iter().filter(|o| o.kind != ObsKind::Head).count();
        if !o.quiet {
            write!(out, "{seq} {:.3}", pts.0 as f64 / 1e6)?;
            for o in &obs {
                let k = match o.kind {
                    ObsKind::Head => 'H',
                    ObsKind::TrailStart => 'S',
                    ObsKind::TrailEnd => 'E',
                    ObsKind::TrailCut => 'C',
                };
                write!(out, " {}{k}{:.1}", o.lane, o.y)?;
            }
            writeln!(out)?;
        }
        if let Some(dir) = o.debug_png
            && i % o.every.max(1) == 0
        {
            debug_png(&dir.join(format!("{i:06}.png")), &f, &det, &obs)?;
        }
        std::mem::swap(&mut prev, &mut obs);
    }
    while let Some(t) = sched.next_due() {
        clock.sleep_until(t);
        sched.fire_due(clock.now(), true, &mut sink, &mut sev);
    }
    out.flush()?;
    println!("frames    {n}, head obs {heads}, trail obs {trails}");
    if eng.is_some() {
        let downs = sink
            .log
            .iter()
            .filter(|x| matches!(x.1, KeyAction::Press(_)))
            .count();
        println!(
            "keys      {downs} key downs, {} key ups, {} dropped by the scheduler",
            sink.log.len() - downs,
            sched.dropped
        );
    }
    if let Some(eng) = &eng {
        println!(
            "plan      presses per lane {presses:?} ({} total, {holds} planned as holds), missed by engine {}, scroll {:.3} px/ms, speed changes {}",
            presses.iter().sum::<u32>(),
            eng.planner.missed_by_engine,
            eng.tracker.velocity().unwrap_or(0.0).abs(),
            eng.tracker.scroll_changes
        );
    }
    println!(
        "motion    {} up / {} down (heads moving {:?})",
        votes[0],
        votes[1],
        if votes[0] >= votes[1] {
            Scroll::Up
        } else {
            Scroll::Down
        }
    );
    println!(
        "detect us p50 {:.0} p99 {:.0} max {:.0}",
        pct(&mut times, 50.0),
        pct(&mut times, 99.0),
        pct(&mut times, 100.0)
    );
    Ok(())
}

type Start = (
    Layout,
    Colours,
    [Vec<u8>; 4],
    (PlanConfig, Option<StylePlayer>),
);

/// Find receptors and learn colours on the clip's first `upto` frames (default `detect`).
fn calibrate(
    reader: &mut ClipReader,
    h: &ClipHeader,
    upto: usize,
    cfg: &DetectConfig,
) -> Result<Start, Box<dyn std::error::Error>> {
    let snaps = spread(reader, 25, upto)?;
    let views: Vec<FrameView> = snaps.iter().map(|f| f.view()).collect();
    let groups = find_groups(&find_blobs(&views, h.height), h.height);
    for (i, g) in groups.iter().enumerate() {
        println!(
            "group {i}: centre ({:.1}, {:.1}) size {:.1} pitch {:.1}",
            g.cx, g.cy, g.diam, g.pitch
        );
    }
    let ours = pick_ours(&groups, h.width, Side::Centre)
        .ok_or("no receptor group found, or the centre rule could not pick ours")?;
    let g = &groups[ours];
    let guess = guess_scroll(g, h.height);
    let mut layout = layout_for(g, &views, guess, [h.width, h.height], cfg);
    let learner = match learn(reader, &layout, cfg, upto) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("learning with scroll {guess:?} failed ({e}), trying the other way");
            layout.scroll = if guess == Scroll::Up {
                Scroll::Down
            } else {
                Scroll::Up
            };
            learn(reader, &layout, cfg, upto)?
        }
    };
    let colours = learner.finish()?;
    println!("side      group {ours} (centre rule)");
    println!("scroll    {:?} (guessed {guess:?})", layout.scroll);
    for (l, r) in layout.receptors.iter().enumerate() {
        println!(
            "lane {l}    receptor ({:.1}, {:.1}) run {:.0} head {:?} tol {} len {:.0}",
            r[0],
            r[1],
            layout.run_len[l],
            colours.heads[l],
            colours.head_tol[l],
            colours.head_len[l]
        );
    }
    Ok((layout, colours, learner.bg, (PlanConfig::default(), None)))
}

/// Start like a live run (`arrowhead_runner::calibrate` with a saved layout): the profile's
/// layout with its nudge, its colours, the background from the first 1.5 s of the clip, and the
/// config's timing, the profile's offset and the config's style (fixed seed).
fn from_profile(
    reader: &mut ClipReader,
    name: &str,
    n: usize,
) -> Result<Start, Box<dyn std::error::Error>> {
    let mut cfg = arrowhead_runner::load_config().map_err(|e| e.to_string())?;
    if !name.is_empty() {
        cfg.profile = name.to_string();
    }
    let path = arrowhead_runner::profile_path(&cfg).ok_or("no config dir")?;
    let prof: Profile = load_toml(&path)?;
    let (Some(saved), Some(colours)) = (prof.layout.clone(), prof.colours.clone()) else {
        return Err(format!(
            "profile {} has no saved layout and colours: run Calibrate first",
            path.display()
        )
        .into());
    };
    let layout = prof.nudge.apply(&saved);
    println!(
        "profile   {} (offset {:+.2} ms)",
        path.display(),
        prof.latency.game_bias_ms
    );
    println!("scroll    {:?}", layout.scroll);
    for (l, r) in layout.receptors.iter().enumerate() {
        println!(
            "lane {l}    receptor ({:.1}, {:.1}) head {:?} tol {} len {:.0}",
            r[0], r[1], colours.heads[l], colours.head_tol[l], colours.head_len[l]
        );
    }
    // background: 21 frames over the first 1.5 s, as `bg_learner` takes live
    let t0 = reader.read(0)?.pts;
    let mut upto = 1;
    while upto < n && reader.read(upto)?.pts.ms_since(t0) < 1500.0 {
        upto += 1;
    }
    let bg = spread(reader, 21, upto.max(2))?;
    let views: Vec<FrameView> = bg.iter().map(|f| f.view()).collect();
    let learner = ColourLearner::new(layout.clone(), DetectConfig::default(), &views);
    let plan = PlanConfig::from_config(&cfg, f64::from(prof.latency.game_bias_ms));
    let style = arrowhead_core::style::find_style(config_dir().as_deref(), &cfg.style);
    println!("style     {}", style.name);
    let player = (!style.is_max()).then(|| StylePlayer::new(style, &cfg.game, 1, 600));
    Ok((layout, colours, learner.bg, (plan, player)))
}

/// Frame with strips (yellow), receptor centres (white) and observations
/// (head: lane colour bar, trail end: white bar, trail start: grey bar).
fn debug_png(path: &Path, f: &FrameView, det: &Detector, obs: &[NoteObs]) -> Res {
    let (w, h) = (f.region.w as usize, f.region.h as usize);
    let mut rgb = vec![0u8; w * h * 3];
    for y in 0..h {
        for x in 0..w {
            let p = &f.data[y * f.stride + x * 4..];
            rgb[(y * w + x) * 3..][..3].copy_from_slice(&[p[2], p[1], p[0]]);
        }
    }
    let mut put = |x: i32, y: i32, c: [u8; 3]| {
        let (x, y) = (x - f.region.x, y - f.region.y);
        if x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h {
            rgb[(y as usize * w + x as usize) * 3..][..3].copy_from_slice(&c);
        }
    };
    for l in 0..4 {
        let s = det.strip(l);
        for y in s.y..s.y + s.h as i32 {
            put(s.x, y, [255, 220, 0]);
            put(s.x + s.w as i32 - 1, y, [255, 220, 0]);
        }
        let [cx, cy] = det.layout.receptors[l];
        for d in -6..=6 {
            put(cx as i32 + d, cy as i32, [255, 255, 255]);
            put(cx as i32, cy as i32 + d, [255, 255, 255]);
        }
    }
    for o in obs {
        let s = det.strip(o.lane as usize);
        let c = match o.kind {
            ObsKind::Head => [255, 0, 255],
            ObsKind::TrailEnd => [255, 255, 255],
            ObsKind::TrailStart | ObsKind::TrailCut => [128, 128, 128],
        };
        for x in s.x - 12..s.x + s.w as i32 + 12 {
            put(x, o.y.round() as i32, c);
            put(x, o.y.round() as i32 + 1, c);
        }
    }
    let mut enc = png::Encoder::new(BufWriter::new(File::create(path)?), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header()?;
    w.write_image_data(&rgb)?;
    w.finish()?;
    Ok(())
}
