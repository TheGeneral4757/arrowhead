//! `detect <clip>`: calibrate on the clip's first seconds, then print observations per frame.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::time::Instant;

use arrowhead_capture::clip::ClipReader;
use arrowhead_core::config::{Scroll, Side};
use arrowhead_core::detect::calib::{
    ColourLearner, find_blobs, find_groups, guess_scroll, layout_for, pick_ours,
};
use arrowhead_core::detect::{DetectConfig, Detector, Layout, NoteObs, ObsKind};
use arrowhead_core::engine::Engine;
use arrowhead_core::frame::{FrameView, OwnedFrame};
use arrowhead_core::plan::{Cmd, PlanConfig};

use crate::{Res, pct};

pub struct Opts<'a> {
    pub clip: &'a Path,
    pub learn_s: f32,
    pub debug_png: Option<&'a Path>,
    pub every: usize,
    pub quiet: bool,
    /// Also run tracker + planner (dry, no keys) and report what would be pressed.
    pub plan: bool,
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

    let snaps = spread(&mut reader, 9, upto)?;
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
    let mut layout = layout_for(g, &views, guess, [h.width, h.height], &cfg);
    let learner = match learn(&mut reader, &layout, &cfg, upto) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("learning with scroll {guess:?} failed ({e}), trying the other way");
            layout.scroll = if guess == Scroll::Up {
                Scroll::Down
            } else {
                Scroll::Up
            };
            learn(&mut reader, &layout, &cfg, upto)?
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
    println!("trail     {:?} tol {}", colours.trail, colours.trail_tol);
    let region = layout.region(&cfg);
    println!(
        "region    {},{},{},{}",
        region.x, region.y, region.w, region.h
    );

    let mut det = Detector::new(layout.clone(), cfg.clone(), colours.clone());
    let mut planner_det = Detector::new(layout.clone(), cfg, colours);
    for (l, bg) in learner.bg.iter().enumerate() {
        det.set_background(l, bg);
        planner_det.set_background(l, bg);
    }
    let mut eng = o
        .plan
        .then(|| Engine::new(planner_det, PlanConfig::default()));
    let (mut cmds, mut presses, mut holds) = (Vec::new(), [0u32; 4], 0u32);
    let mut buf = Vec::new();
    let mut obs: Vec<NoteObs> = Vec::with_capacity(64);
    let mut prev: Vec<NoteObs> = Vec::with_capacity(64);
    let mut times = Vec::with_capacity(n);
    let (mut heads, mut trails, mut votes) = (0usize, 0usize, [0i64; 2]);
    let out = std::io::stdout();
    let mut out = BufWriter::new(out.lock());
    for i in 0..n {
        let (pts, seq) = reader.read_into(i, &mut buf)?;
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
            eng.on_frame(&f, pts.add_ms(3.0), &mut cmds);
            for c in &cmds {
                if let Cmd::Press {
                    lane, at, release, ..
                } = *c
                {
                    presses[lane as usize] += 1;
                    if release.ms_since(at) > 60.0 {
                        holds += 1;
                    }
                    if !o.quiet {
                        writeln!(out, "press lane {lane} at {:.1} ms", at.ms())?;
                    }
                }
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
    out.flush()?;
    println!("frames    {n}, head obs {heads}, trail obs {trails}");
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
