//! Scan-thread pipeline: detect -> track -> plan (spec 04). Pure; the runtime feeds it frames
//! and hands its commands to the scheduler.

use crate::detect::{Detector, NoteObs};
use crate::frame::FrameView;
use crate::plan::{Cmd, PlanConfig, Planner};
use crate::time::Nanos;
use crate::track::Tracker;

pub struct Engine {
    pub det: Detector,
    pub tracker: Tracker,
    pub planner: Planner,
    obs: Vec<NoteObs>,
    /// The last frame's observations, to spot a repeated frame.
    prev: Vec<NoteObs>,
    /// Frames skipped by the tracker as repeats.
    pub repeats: u64,
    last_now: Option<Nanos>,
    /// Smoothed frame interval, ms.
    pub frame_ms: f64,
}

impl Engine {
    pub fn new(det: Detector, plan: PlanConfig) -> Self {
        let tracker = Tracker::new(&det.layout, det.colours.head_len);
        Self {
            det,
            tracker,
            planner: Planner::new(plan),
            obs: Vec::with_capacity(64),
            prev: Vec::with_capacity(64),
            repeats: 0,
            last_now: None,
            frame_ms: 16.7,
        }
    }

    /// `now` is when the frame reached us; commands are appended to `out`.
    pub fn on_frame(&mut self, f: &FrameView, now: Nanos, out: &mut Vec<Cmd>) {
        std::mem::swap(&mut self.obs, &mut self.prev);
        self.det.detect(f, &mut self.obs);
        // A game frame shown twice: every head and trail end exactly where it was. Its new pts
        // would tell the tracker the notes stopped (spec 13, 4.2), so it only plans.
        if repeated(&self.prev, &self.obs) {
            self.repeats += 1;
        } else {
            self.tracker.update(f.pts, &self.obs);
        }
        if let Some(prev) = self.last_now.replace(now) {
            let dt = now.ms_since(prev).clamp(1.0, 50.0);
            self.frame_ms += 0.1 * (dt - self.frame_ms);
        }
        self.planner.plan(&self.tracker, now, self.frame_ms, out);
    }

    pub fn obs(&self) -> &[NoteObs] {
        &self.obs
    }
}

/// Same observations as the last frame, to the sub-pixel, with something that moves (a head or
/// a trail end; trail starts and cuts sit still at the strip's edges during a hold).
fn repeated(prev: &[NoteObs], now: &[NoteObs]) -> bool {
    use crate::detect::ObsKind;
    now.len() == prev.len()
        && now
            .iter()
            .any(|o| matches!(o.kind, ObsKind::Head | ObsKind::TrailEnd))
        && now
            .iter()
            .zip(prev)
            .all(|(a, b)| a.lane == b.lane && a.kind == b.kind && a.y == b.y)
}

#[cfg(test)]
mod tests {
    //! M4 synthetic acceptance (spec 04): Max on a virtual clock. Every press within 2 ms of
    //! the true crossing (2.5 ms at 60 Hz), holds released within 5 ms of their end, no extra
    //! presses, every press released.

    use std::sync::Arc;

    use super::*;
    use crate::config::{Game, Scroll};
    use crate::detect::synth_tests::calibrate;
    use crate::frame::Rect;
    use crate::input::{KeyAction, RecordingSink};
    use crate::sched::Scheduler;
    use crate::style::{Style, StylePlayer};
    use crate::testkit::*;
    use crate::time::{Clock, VirtualClock};

    /// Synthetic pts are chart time + 10 s.
    fn pts_of(ms: f64) -> Nanos {
        Nanos::from_ms(ms + 10_000.0)
    }

    #[derive(Debug, Default)]
    #[allow(dead_code)] // fields are read through Debug in the failure message
    struct Score {
        notes: usize,
        unpressed: usize,
        /// Middle notes of 3+ overlapping heads (known limit, see spec 03).
        hidden_unpressed: usize,
        extra: usize,
        worst_press_ms: f64,
        /// Notes whose head overlaps a neighbour's (slow jacks).
        worst_overlap_ms: f64,
        holds: usize,
        worst_release_ms: f64,
        missed_by_engine: u32,
        presses: usize,
        style_misses: u32,
        /// Presses held under 8 ms: a jack whose notes were pressed out of order.
        short_presses: usize,
    }

    fn worst(cur: &mut f64, err: f64) {
        if err.abs() > cur.abs() {
            *cur = err;
        }
    }

    fn suite(seed: u64, fps: f64, speed: f64, scroll: Scroll, notes: usize) -> Score {
        suite_with(seed, fps, speed, scroll, notes, None)
    }

    fn suite_with(
        seed: u64,
        fps: f64,
        speed: f64,
        scroll: Scroll,
        notes: usize,
        style: Option<StylePlayer>,
    ) -> Score {
        suite_full(
            seed,
            fps,
            speed,
            scroll,
            notes,
            style,
            ([0.0; 4], [0.0; 3]),
            0.0,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn suite_full(
        seed: u64,
        fps: f64,
        speed: f64,
        scroll: Scroll,
        notes: usize,
        style: Option<StylePlayer>,
        corr: ([f64; 4], [f64; 3]),
        repeat: f64,
    ) -> Score {
        let mut rng = Rng(seed);
        let mut s = Scene::random(&mut rng, 1280, 720, scroll, speed);
        s.fps = fps;
        s.repeat = repeat;
        let chart = chart(&mut rng, notes, 3000.0);
        let cal = calibrate(&s, &chart, &mut rng);
        let region = cal.layout.region(&cal.det.cfg);
        let strips: Vec<Rect> = (0..4).map(|l| cal.det.strip(l)).collect();
        let mut eng = Engine::new(cal.det, PlanConfig::default());
        if let Some(p) = style {
            eng.planner.set_style(p);
        }
        eng.planner.set_corrections(corr.0, corr.1);
        let clock = Arc::new(VirtualClock::default());
        let mut sink = RecordingSink::new(clock.clone());
        let mut sched = Scheduler::new(15_000.0);
        let mut cmds = Vec::new();
        let mut ev = Vec::new();
        let end = chart.last().map_or(0.0, |n| n.t + n.hold) + 500.0;
        let mut drain = |sched: &mut Scheduler, sink: &mut RecordingSink, upto: Nanos| {
            while let Some(t) = sched.next_due().filter(|&t| t <= upto) {
                clock.sleep_until(t);
                sched.fire_due(clock.now(), true, sink, &mut ev);
            }
            clock.set(upto);
        };
        let mut shown = 0.0;
        for (t, pts) in s.frames(&mut rng, 0.0, end) {
            // A repeated frame shows the last one's picture with its own pts.
            if repeat == 0.0 || rng.f64() >= repeat {
                shown = t;
            }
            // The frame reaches us 3 ms after it was shown.
            let now = pts_of(t + 3.0);
            drain(&mut sched, &mut sink, now);
            let mut f = s.render(&chart, shown, region, Some(&strips));
            f.pts = pts;
            cmds.clear();
            eng.on_frame(&f.view(), now, &mut cmds);
            cmds.iter().for_each(|&c| sched.submit(c));
        }
        drain(&mut sched, &mut sink, pts_of(end + 20_000.0));

        let mut sc = Score {
            notes: chart.len(),
            missed_by_engine: eng.planner.missed_by_engine,
            style_misses: eng.planner.style_misses,
            ..Default::default()
        };
        for lane in 0..4u8 {
            let times = |a: KeyAction| -> Vec<Nanos> {
                sink.log.iter().filter(|x| x.1 == a).map(|x| x.0).collect()
            };
            let (presses, releases) = (
                times(KeyAction::Press(lane)),
                times(KeyAction::Release(lane)),
            );
            assert_eq!(
                presses.len(),
                releases.len(),
                "lane {lane}: every press released"
            );
            let mut used = vec![false; presses.len()];
            let gap = s.size as f64 / s.speed;
            let lane_notes = || chart.iter().filter(|m| m.lane == lane);
            for n in lane_notes() {
                let cross = pts_of(n.t);
                let off = |i: usize| presses[i].ms_since(cross).abs();
                let best = (0..presses.len())
                    .filter(|&i| !used[i])
                    .min_by(|&a, &b| off(a).total_cmp(&off(b)))
                    .filter(|&i| off(i) <= 20.0);
                match best {
                    Some(i) => {
                        used[i] = true;
                        // Notes overlapping a neighbour are placed from a run end, not a centroid.
                        let overlapped =
                            lane_notes().any(|m| m.t != n.t && (m.t - n.t).abs() < gap);
                        let w = if overlapped {
                            &mut sc.worst_overlap_ms
                        } else {
                            &mut sc.worst_press_ms
                        };
                        worst(w, presses[i].ms_since(cross));
                        if n.hold > 0.0 {
                            sc.holds += 1;
                            worst(
                                &mut sc.worst_release_ms,
                                releases[i].ms_since(pts_of(n.t + n.hold)),
                            );
                        }
                    }
                    None if lane_notes().any(|m| m.t < n.t && n.t - m.t < gap)
                        && lane_notes().any(|m| m.t > n.t && m.t - n.t < gap) =>
                    {
                        sc.hidden_unpressed += 1
                    }
                    None => sc.unpressed += 1,
                }
            }
            sc.short_presses += presses
                .iter()
                .zip(&releases)
                .filter(|(p, r)| r.ms_since(**p) < 7.9)
                .count();
            sc.extra += used.iter().filter(|u| !**u).count();
            sc.presses += presses.len();
        }
        sc
    }

    #[test]
    fn lane_and_class_corrections_shift_presses() {
        let base = suite(70, 144.0, 2.0, Scroll::Up, 60);
        let lane = suite_full(
            70,
            144.0,
            2.0,
            Scroll::Up,
            60,
            None,
            ([-4.0; 4], [0.0; 3]),
            0.0,
        );
        let class = suite_full(
            70,
            144.0,
            2.0,
            Scroll::Up,
            60,
            None,
            ([0.0; 4], [-4.0; 3]),
            0.0,
        );
        assert_eq!(lane.short_presses, 0, "{lane:?}");
        assert_eq!(lane.unpressed, base.unpressed, "{lane:?}");
        for sc in [&lane, &class] {
            let d = sc.worst_press_ms - base.worst_press_ms;
            assert!((-6.5..=-1.5).contains(&d), "shift {d}: {sc:?} vs {base:?}");
        }
    }

    #[test]
    fn repeated_frames_keep_timing() {
        // 15% of frames show the previous picture again (spec 13, 4.2).
        for (seed, fps) in [(80, 144.0), (81, 165.0), (82, 60.0)] {
            let sc = suite_full(
                seed,
                fps,
                2.0,
                Scroll::Up,
                150,
                None,
                ([0.0; 4], [0.0; 3]),
                0.15,
            );
            eprintln!("{fps} Hz, 15% repeats: {sc:?}");
            let tol = if fps < 100.0 { 2.5 } else { 2.0 };
            assert!(
                sc.unpressed + sc.extra == 0
                    && sc.worst_press_ms.abs() <= tol
                    && sc.worst_release_ms.abs() <= 5.0,
                "{fps} Hz: {sc:?}"
            );
        }
    }

    #[test]
    fn max_style_changes_nothing() {
        let a = suite(70, 144.0, 2.0, Scroll::Up, 60);
        let b = suite_with(
            70,
            144.0,
            2.0,
            Scroll::Up,
            60,
            Some(StylePlayer::new(Style::max(), &Game::default(), 1, 60)),
        );
        assert_eq!(format!("{a:?}"), format!("{b:?}"));
    }

    #[test]
    fn styled_replay_skips_misses_and_releases_everything() {
        let st = Style {
            miss_floor: 0.1,
            ..Style::builtin("Casual friend").unwrap()
        };
        let p = StylePlayer::new(st, &Game::default(), 5, 150);
        let sc = suite_with(71, 144.0, 2.0, Scroll::Up, 150, Some(p));
        // Presses = notes - style misses (a few hidden middle jack notes aside); suite() already
        // asserted every press has a release.
        assert!(sc.style_misses > 5, "{sc:?}");
        assert!(
            sc.presses + sc.style_misses as usize <= sc.notes + sc.extra,
            "{sc:?}"
        );
        assert!(
            sc.presses >= sc.notes - sc.style_misses as usize - 8,
            "{sc:?}"
        );
    }

    #[test]
    fn styled_jacks_keep_note_order() {
        // Rusty's wide errors would swap the notes of 50-100 ms jacks without the lane floor.
        for seed in [72, 90, 91] {
            let st = Style::builtin("Rusty").unwrap();
            let p = StylePlayer::new(st, &Game::default(), seed, 200).with_fixed_k(1.5);
            let sc = suite_with(seed, 144.0, 2.0, Scroll::Up, 200, Some(p));
            assert_eq!(sc.short_presses, 0, "seed {seed}: {sc:?}");
        }
    }

    #[test]
    fn max_replay_hits_every_crossing() {
        let mut seed = 50;
        let mut bad = Vec::new();
        for fps in [60.0, 144.0, 240.0] {
            for (i, speed) in [1.0, 2.0, 3.0, 4.0].into_iter().enumerate() {
                let scroll = if i % 2 == 0 { Scroll::Up } else { Scroll::Down };
                let sc = suite(seed, fps, speed, scroll, 150);
                eprintln!("{fps} Hz {speed} px/ms {scroll:?}: {sc:?}");
                // 2 ms, except 60 Hz: a press is committed two frames (33 ms) ahead there.
                let tol = if fps < 100.0 { 2.5 } else { 2.0 };
                if sc.unpressed + sc.extra > 0
                    || sc.worst_press_ms.abs() > tol
                    || sc.worst_overlap_ms.abs() > 3.0
                    || sc.worst_release_ms.abs() > 5.0
                {
                    bad.push((fps, speed, sc));
                }
                seed += 1;
            }
        }
        assert!(bad.is_empty(), "{bad:#?}");
    }
}
