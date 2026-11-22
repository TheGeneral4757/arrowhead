//! Planning (spec 04): tracks -> press and release times.

use crate::config::Config;
use crate::time::Nanos;
use crate::track::Tracker;

#[derive(Clone, Debug)]
pub struct PlanConfig {
    pub commit_ms: f64,
    pub late_limit_ms: f64,
    pub tap_hold_ms: f64,
    pub min_gap_ms: f64,
    pub max_hold_ms: f64,
    /// Added to every crossing (negative = send earlier): latency compensation + game bias.
    pub offset_ms: f64,
}

impl PlanConfig {
    pub fn from_config(c: &Config, offset_ms: f64) -> Self {
        Self {
            commit_ms: c.timing.commit_ms as f64,
            late_limit_ms: c.timing.late_limit_ms as f64,
            tap_hold_ms: c.timing.tap_hold_ms as f64,
            min_gap_ms: c.timing.min_gap_ms as f64,
            max_hold_ms: c.safety.max_hold_ms as f64,
            offset_ms,
        }
    }
}

impl Default for PlanConfig {
    fn default() -> Self {
        Self::from_config(&Config::default(), 0.0)
    }
}

/// To the scheduler. Every press carries its release (spec 04 invariant).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Cmd {
    Press {
        id: u32,
        lane: u8,
        at: Nanos,
        release: Nanos,
    },
    MoveRelease {
        id: u32,
        release: Nanos,
    },
}

#[derive(Clone, Copy, Debug)]
struct Commit {
    id: u32,
    lane: u8,
    cross: f64,
    /// NaN when skipped (found too late).
    press: f64,
    release: f64,
}

pub struct Planner {
    pub cfg: PlanConfig,
    commits: Vec<Commit>,
    pub missed_by_engine: u32,
    pub committed: u32,
}

/// Same note seen as two tracks (one broke up): crossings closer than this are one note.
const SAME_NOTE_MS: f64 = 25.0;

impl Planner {
    pub fn new(cfg: PlanConfig) -> Self {
        Self {
            cfg,
            commits: Vec::with_capacity(256),
            missed_by_engine: 0,
            committed: 0,
        }
    }

    /// Release time for a note in `lane` crossing at `cross` and pressed at `press`.
    fn release_for(&self, tr: &Tracker, lane: u8, cross: f64, press: f64, now_ms: f64) -> f64 {
        let c = &self.cfg;
        let mut next_head = f64::INFINITY;
        for t in tr
            .tracks()
            .iter()
            .filter(|t| t.confirmed && !t.tail && t.lane == lane)
        {
            if let Some(x) = tr.cross_ms(t)
                && x > cross + SAME_NOTE_MS
            {
                next_head = next_head.min(x);
            }
        }
        // Hold: the nearest trail end behind this head and before the next head.
        let tail = tr
            .tracks()
            .iter()
            .filter(|t| t.confirmed && t.tail && t.lane == lane)
            .filter_map(|t| tr.cross_ms(t))
            .filter(|&x| x > cross && x < next_head)
            .fold(f64::INFINITY, f64::min);
        // Hold whose end is not visible yet: before the press its trail runs off the strip; after
        // it, the trail stays attached to the receptor. Keep holding while that is seen.
        let open = next_head.is_infinite()
            && (tr.trail_cut(lane) || (now_ms > press && now_ms - tr.trail_seen(lane) < 50.0));
        let mut rel = if tail.is_finite() {
            tail + c.offset_ms
        } else if open {
            press + c.max_hold_ms
        } else {
            (press + c.tap_hold_ms).max(now_ms.min(press + c.max_hold_ms))
        };
        // Jack: leave min_gap before the next press in this lane.
        let next_press = next_head + c.offset_ms;
        if rel > next_press - c.min_gap_ms {
            rel = next_press - c.min_gap_ms;
        }
        rel.max(press + 1.0).min(press + c.max_hold_ms)
    }

    /// `frame_ms`: the current frame interval. A plan is committed when the next frame might
    /// arrive after its commit point, allowing one dropped frame (`press - now < commit_ms +
    /// 2 * frame_ms`).
    pub fn plan(&mut self, tr: &Tracker, now: Nanos, frame_ms: f64, out: &mut Vec<Cmd>) {
        let now_ms = tr.t(now);
        let c = self.cfg.clone();
        let horizon = c.commit_ms + 2.0 * frame_ms;
        for t in tr.tracks().iter().filter(|t| t.confirmed && !t.tail) {
            let Some(cross) = tr.cross_ms(t) else {
                continue;
            };
            if self.commits.iter().any(|k| {
                k.id == t.id || (k.lane == t.lane && (k.cross - cross).abs() < SAME_NOTE_MS)
            }) {
                continue;
            }
            let mut press = cross + c.offset_ms;
            if press - now_ms >= horizon {
                continue;
            }
            if press < now_ms {
                if now_ms - cross > c.late_limit_ms {
                    self.missed_by_engine += 1;
                    self.commits.push(Commit {
                        id: t.id,
                        lane: t.lane,
                        cross,
                        press: f64::NAN,
                        release: f64::NAN,
                    });
                    continue;
                }
                press = now_ms;
            }
            // Chords: snap to another lane's press within 1 ms.
            if let Some(k) = self
                .commits
                .iter()
                .find(|k| k.lane != t.lane && (k.press - press).abs() < 1.0)
            {
                press = k.press;
            }
            let release = self.release_for(tr, t.lane, cross, press, now_ms);
            // The previous note in this lane must be up min_gap before this press.
            for k in self
                .commits
                .iter_mut()
                .filter(|k| k.lane == t.lane && k.press.is_finite())
            {
                if k.release > press - c.min_gap_ms && k.press < press {
                    k.release = (press - c.min_gap_ms).max(k.press + 1.0);
                    out.push(Cmd::MoveRelease {
                        id: k.id,
                        release: tr.to_nanos(k.release),
                    });
                }
            }
            self.commits.push(Commit {
                id: t.id,
                lane: t.lane,
                cross,
                press,
                release,
            });
            self.committed += 1;
            out.push(Cmd::Press {
                id: t.id,
                lane: t.lane,
                at: tr.to_nanos(press),
                release: tr.to_nanos(release),
            });
        }
        // Re-plan releases that are not yet committed (holds found after the press, jacks).
        for i in 0..self.commits.len() {
            let k = self.commits[i];
            if !k.press.is_finite() || k.release - now_ms < horizon {
                continue;
            }
            let mut rel = self.release_for(tr, k.lane, k.cross, k.press, now_ms);
            // Never move past a later press already committed in this lane.
            for n in self
                .commits
                .iter()
                .filter(|n| n.lane == k.lane && n.press > k.press)
            {
                rel = rel.min(n.press - c.min_gap_ms);
            }
            let rel = rel.max(k.press + 1.0);
            if (rel - k.release).abs() > 0.5 {
                self.commits[i].release = rel;
                out.push(Cmd::MoveRelease {
                    id: k.id,
                    release: tr.to_nanos(rel),
                });
            }
        }
        self.commits.retain(|k| {
            !(k.release < now_ms - 2000.0 || (k.press.is_nan() && k.cross < now_ms - 2000.0))
        });
    }
}
