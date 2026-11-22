//! M3 acceptance on synthetic frames (spec 03, 09): calibrate, learn colours, then detect
//! every frame and compare with ground truth.

#![allow(clippy::needless_range_loop)]

use super::calib::*;
use super::*;
use crate::config::{Scroll, Side};
use crate::frame::OwnedFrame;
use crate::testkit::*;

struct Calibrated {
    layout: Layout,
    det: Detector,
}

fn calibrate(s: &Scene, chart: &[Note], rng: &mut Rng) -> Calibrated {
    let full = Rect::new(0, 0, s.w, s.h);
    let snaps: Vec<OwnedFrame> = (0..9)
        .map(|i| s.render(chart, i as f64 * 250.0, full, None))
        .collect();
    let views: Vec<_> = snaps.iter().map(|f| f.view()).collect();
    let blobs = find_blobs(&views, s.h);
    let groups = find_groups(&blobs, s.h);
    let ours = pick_ours(&groups, s.w, Side::Centre).expect("our group");
    let g = &groups[ours];
    let scroll = guess_scroll(g, s.h);
    assert_eq!(scroll, s.scroll);
    let cfg = DetectConfig::default();
    let layout = layout_for(g, &views, scroll, [s.w, s.h], &cfg);
    for l in 0..4 {
        // Row y stands for the pixel centre y + 0.5.
        let dx = layout.receptors[l][0] - s.receptor(l)[0];
        let dy = layout.receptors[l][1] + 0.5 - s.receptor(l)[1];
        assert!(
            dx.abs() <= 1.0 && dy.abs() <= 1.0,
            "lane {l} off by ({dx}, {dy}) ({:?})",
            s.shape
        );
    }

    let region = layout.region(&cfg);
    let strips: Vec<Rect> = (0..4).map(|l| layout.strip(l, &cfg)).collect();
    let bg: Vec<OwnedFrame> = (0..21)
        .map(|i| s.render(chart, i as f64 * 1000.0, region, Some(&strips)))
        .collect();
    let bgv: Vec<_> = bg.iter().map(|f| f.view()).collect();
    let mut learner = ColourLearner::new(layout.clone(), cfg.clone(), &bgv);
    for (t, _) in s.frames(rng, 0.0, 20_000.0) {
        learner.feed(&s.render(chart, t, region, Some(&strips)).view());
        if learner.counts().iter().all(|&c| c >= 40) && learner.trail_runs() >= 20 {
            break;
        }
    }
    let pres = Presence::new(&layout, &bgv, &cfg);
    // t = 0: before the first hit, so no receptor is lit.
    assert!(pres.check(&bgv[0]), "receptors present");
    let blank = OwnedFrame {
        seq: 0,
        pts: Default::default(),
        region,
        data: vec![0; (region.w * region.h * 4) as usize],
    };
    assert!(!pres.check(&blank.view()), "receptors absent");
    let colours = learner.finish().unwrap_or_else(|e| {
        panic!(
            "{e}: {:?} heads {:?} bg {:?} counts {:?}",
            s.shape,
            s.heads,
            s.bg,
            learner.counts()
        )
    });
    let mut det = Detector::new(layout.clone(), cfg, colours);
    for l in 0..4 {
        det.set_background(l, &learner.bg[l]);
    }
    Calibrated { layout, det }
}

#[derive(Default, Debug)]
struct Score {
    notes: usize,
    checked: usize,
    missed: usize,
    false_heads: usize,
    trail_checked: usize,
    trail_missed: usize,
    under_3: usize,
}

fn run_suite(seed: u64, w: u32, h: u32, scroll: Scroll, speed: f64, notes: usize) -> Score {
    let mut rng = Rng(seed);
    let s = Scene::random(&mut rng, w, h, scroll, speed);
    let chart = chart(&mut rng, notes, 1000.0);
    let Calibrated { layout, mut det } = calibrate(&s, &chart, &mut rng);
    let region = layout.region(&det.cfg);
    let strips: Vec<Rect> = (0..4).map(|l| det.strip(l)).collect();
    let (r, size) = (s.size / 2.0, s.size);
    let a = s.approach();
    let tol = 2.0;
    let mut sc = Score {
        notes: chart.len(),
        ..Default::default()
    };
    let mut seen = vec![(0u32, 0u32); chart.len()]; // (checkable frames, matched frames)
    let mut obs = Vec::with_capacity(64);
    let end = chart.last().map_or(0.0, |n| n.t + n.hold) + 200.0;
    for (t, pts) in s.frames(&mut rng, 0.0, end) {
        let mut f = s.render(&chart, t, region, Some(&strips));
        f.pts = pts;
        det.detect(&f.view(), &mut obs);
        let range = s.window_range(&chart, t);
        let notes = &chart[range.clone()];
        let lit = |lane: usize| {
            notes
                .iter()
                .any(|n| n.lane as usize == lane && t >= n.t && t < n.t + n.hold.max(80.0))
        };
        // Offsets from the receptor, along y: truth and detected.
        let off_obs = |o: &NoteObs| o.y - layout.receptors[o.lane as usize][1];
        for lane in 0..4 {
            let strip = strips[lane];
            let ry = s.centre[1];
            let heads: Vec<(usize, f32)> = range
                .clone()
                .filter(|&i| chart[i].lane as usize == lane && t < chart[i].t)
                .map(|i| (i, s.note_y(chart[i].t, t)))
                .collect();
            let inside = |y: f32| {
                y - r >= strip.y as f32 + 1.0 && y + r <= (strip.y + strip.h as i32) as f32 - 1.0
            };
            let lane_obs: Vec<&NoteObs> = obs.iter().filter(|o| o.lane as usize == lane).collect();
            for &(i, y) in &heads {
                let near = heads
                    .iter()
                    .any(|&(_, z)| z != y && (z - ry) * a < (y - ry) * a && (y - z).abs() < size);
                let far = heads
                    .iter()
                    .any(|&(_, z)| z != y && (z - ry) * a > (y - ry) * a && (y - z).abs() < size);
                let in_glow = lit(lane) && (y - ry).abs() < size;
                if !inside(y) || (near && far) || in_glow {
                    continue;
                }
                sc.checked += 1;
                seen[i].0 += 1;
                let hit = lane_obs
                    .iter()
                    .any(|o| o.kind == ObsKind::Head && (off_obs(o) - (y - ry)).abs() <= tol);
                if hit {
                    seen[i].1 += 1;
                } else {
                    sc.missed += 1;
                    if sc.missed <= 5 {
                        eprintln!("missed lane {lane} y {y:.1} t {t:.1} obs {lane_obs:?}");
                    }
                }
            }
            for o in lane_obs.iter().filter(|o| o.kind == ObsKind::Head) {
                let d = off_obs(o);
                let edge =
                    (o.y - strip.y as f32).min((strip.y + strip.h as i32) as f32 - o.y) < r + 2.0;
                let glow = lit(lane) && d.abs() < size;
                let ok = heads.iter().any(|&(_, y)| (d - (y - ry)).abs() <= tol);
                if !ok && !edge && !glow {
                    sc.false_heads += 1;
                    if sc.false_heads <= 5 {
                        eprintln!("false lane {lane} y {:.1} t {t:.1} heads {heads:?}", o.y);
                    }
                }
            }
            for n in notes
                .iter()
                .filter(|n| n.lane as usize == lane && n.hold > 0.0 && t < n.t + n.hold)
            {
                let tail = s.note_y(n.t + n.hold, t);
                let head = if t < n.t { s.note_y(n.t, t) } else { ry };
                let beyond = (tail - head) * a - if t < n.t { r } else { r + 2.0 };
                if beyond < 3.0 || !inside(tail) {
                    continue;
                }
                sc.trail_checked += 1;
                let hit = lane_obs.iter().any(|o| {
                    o.kind == ObsKind::TrailEnd && (off_obs(o) - (tail - ry)).abs() <= tol + 0.5
                });
                if !hit {
                    sc.trail_missed += 1;
                    if sc.trail_missed <= 5 {
                        eprintln!(
                            "trail end missed lane {lane} tail {tail:.1} t {t:.1} obs {lane_obs:?}"
                        );
                    }
                }
            }
        }
    }
    sc.under_3 = seen.iter().filter(|&&(c, m)| m < c.min(3)).count();
    sc
}

#[test]
fn synthetic_calibration_all_resolutions() {
    for (i, &(w, h)) in [(1280, 720), (1920, 1080), (2560, 1440)].iter().enumerate() {
        for scroll in [Scroll::Up, Scroll::Down] {
            let mut rng = Rng(100 + i as u64);
            let s = Scene::random(&mut rng, w, h, scroll, 1.0);
            let chart = chart(&mut rng, 200, 1000.0);
            calibrate(&s, &chart, &mut rng);
        }
    }
}

#[test]
fn synthetic_detection_10k_notes() {
    let mut total = Score::default();
    let mut seed = 1;
    for scroll in [Scroll::Up, Scroll::Down] {
        for speed in [0.4, 1.0, 2.0, 3.0] {
            let sc = run_suite(seed, 1280, 720, scroll, speed, 1250);
            eprintln!("seed {seed} {scroll:?} {speed}: {sc:?}");
            seed += 1;
            total.notes += sc.notes;
            total.checked += sc.checked;
            total.missed += sc.missed;
            total.false_heads += sc.false_heads;
            total.trail_checked += sc.trail_checked;
            total.trail_missed += sc.trail_missed;
            total.under_3 += sc.under_3;
        }
    }
    eprintln!("total {total:?}");
    assert!(total.notes >= 10_000);
    assert_eq!(
        (
            total.missed,
            total.false_heads,
            total.trail_missed,
            total.under_3
        ),
        (0, 0, 0, 0)
    );
}
