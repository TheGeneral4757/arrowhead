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
            last_now: None,
            frame_ms: 16.7,
        }
    }

    /// `now` is when the frame reached us; commands are appended to `out`.
    pub fn on_frame(&mut self, f: &FrameView, now: Nanos, out: &mut Vec<Cmd>) {
        self.det.detect(f, &mut self.obs);
        self.tracker.update(f.pts, &self.obs);
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
        let mut rng = Rng(seed);
        let mut s = Scene::random(&mut rng, 1280, 720, scroll, speed);
        s.fps = fps;
        let chart = chart(&mut rng, notes, 3000.0);
        let cal = calibrate(&s, &chart, &mut rng);
        let region = cal.layout.region(&cal.det.cfg);
        let strips: Vec<Rect> = (0..4).map(|l| cal.det.strip(l)).collect();
        let mut eng = Engine::new(cal.det, PlanConfig::default());
        if let Some(p) = style {
            eng.planner.set_style(p);
        }
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
        for (t, pts) in s.frames(&mut rng, 0.0, end) {
            // The frame reaches us 3 ms after it was shown.
            let now = pts_of(t + 3.0);
            drain(&mut sched, &mut sink, now);
            let mut f = s.render(&chart, t, region, Some(&strips));
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
            sc.extra += used.iter().filter(|u| !**u).count();
            sc.presses += presses.len();
        }
        sc
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
