//! Planning (spec 04): tracks -> press and release times.

use crate::config::Config;
use crate::style::{NoteCtx, NoteDraw, StylePlayer, chord_term, jack_term, stream_term};
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
    /// Style error added to the hold end (0 for Max and taps).
    rel_err: f64,
}

/// A note the style model has already drawn for, before it is committed.
#[derive(Clone, Copy, Debug)]
struct Pending {
    id: u32,
    lane: u8,
    cross: f64,
    draw: NoteDraw,
}

const PENDING_CAP: usize = 128;
/// Shortest hold a jack press keeps before the next press (spec 04, jacks).
const MIN_HELD_MS: f64 = 8.0;

pub struct Planner {
    pub cfg: PlanConfig,
    commits: Vec<Commit>,
    pub missed_by_engine: u32,
    pub committed: u32,
    /// Notes the style left unpressed.
    pub style_misses: u32,
    style: Option<StylePlayer>,
    pending: Vec<Pending>,
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
            style_misses: 0,
            style: None,
            pending: Vec::with_capacity(PENDING_CAP),
        }
    }

    /// Plays with a style (spec 06). Max keeps the planner exactly as without a style.
    pub fn set_style(&mut self, p: StylePlayer) {
        self.style = (!p.is_max()).then_some(p);
        self.pending.clear();
    }

    pub fn style(&self) -> Option<&StylePlayer> {
        self.style.as_ref()
    }

    /// Accuracy shown by the game (percent), for the controller; `None` uses our own estimate.
    pub fn feed_accuracy(&mut self, measured: Option<f64>) {
        if let Some(s) = self.style.as_mut() {
            s.feed_accuracy(measured);
        }
    }

    /// Chord, jack and stream terms for a note crossing at `cross` in `lane`, from the notes
    /// the tracker can see (lookahead) and the ones already planned.
    fn note_ctx(&self, tr: &Tracker, id: u32, lane: u8, cross: f64) -> NoteCtx {
        let mut chord = 1usize;
        let mut prev = f64::NEG_INFINITY;
        let mut count = 0usize;
        let mut visit = |oid: u32, olane: u8, x: f64| {
            if oid == id {
                return;
            }
            if olane != lane && (x - cross).abs() <= 5.0 {
                chord += 1;
            }
            if olane == lane && x < cross - SAME_NOTE_MS {
                prev = prev.max(x);
            }
            if x > cross - 1000.0 && x <= cross {
                count += 1;
            }
        };
        for t in tr.tracks().iter().filter(|t| t.confirmed && !t.tail) {
            if let Some(x) = tr.cross_ms(t) {
                visit(t.id, t.lane, x);
            }
        }
        // Planned notes whose track is gone (just played).
        for k in self
            .commits
            .iter()
            .filter(|k| !tr.tracks().iter().any(|t| t.id == k.id))
        {
            visit(k.id, k.lane, k.cross);
        }
        NoteCtx {
            t_ms: cross,
            c: chord_term(chord),
            j: jack_term(cross - prev),
            s: stream_term((count + 1) as f64),
        }
    }

    /// Earliest press for a styled note in `lane` crossing at `cross`: after the earlier notes
    /// of that lane (their committed or planned press, 8 ms held, `min_gap`), so style errors
    /// never swap the notes of a jack. Max needs none: its presses keep crossing order.
    fn lane_floor(&self, lane: u8, cross: f64) -> f64 {
        let c = &self.cfg;
        self.pending
            .iter()
            .filter(|p| p.lane == lane && p.cross < cross - SAME_NOTE_MS && !p.draw.miss)
            .map(|p| {
                self.commits
                    .iter()
                    .find(|k| k.id == p.id)
                    .map_or(p.cross + c.offset_ms + p.draw.error_ms, |k| k.press)
            })
            .filter(|x| x.is_finite())
            .fold(f64::NEG_INFINITY, f64::max)
            + MIN_HELD_MS
            + c.min_gap_ms
    }

    /// The style draw for a track: made once, when the note is first planned.
    fn draw_for(&mut self, tr: &Tracker, id: u32, lane: u8, cross: f64) -> NoteDraw {
        if let Some(p) = self
            .pending
            .iter()
            .find(|p| p.id == id || (p.lane == lane && (p.cross - cross).abs() < SAME_NOTE_MS))
        {
            return p.draw;
        }
        let ctx = self.note_ctx(tr, id, lane, cross);
        let draw = self
            .style
            .as_mut()
            .map_or_else(NoteDraw::default, |s| s.note(ctx));
        if self.pending.len() == PENDING_CAP {
            self.pending.remove(0);
        }
        self.pending.push(Pending {
            id,
            lane,
            cross,
            draw,
        });
        draw
    }

    /// Release time for a note in `lane` crossing at `cross` and pressed at `press`.
    fn release_for(
        &self,
        tr: &Tracker,
        lane: u8,
        cross: f64,
        press: f64,
        now_ms: f64,
        rel_err: f64,
    ) -> f64 {
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
            let end = tail + c.offset_ms;
            // Style release error; a hold never ends before 60% of its length.
            if rel_err != 0.0 && end > press {
                (end + rel_err).max(press + 0.6 * (end - press))
            } else {
                end
            }
        } else if open {
            press + c.max_hold_ms
        } else {
            (press + c.tap_hold_ms).max(now_ms.min(press + c.max_hold_ms))
        };
        // Jack: leave min_gap before the next press in this lane. Styled, that press carries
        // its own error (none if the style misses it) and is never before this one + 8 ms + gap.
        let mut next_press = next_head + c.offset_ms;
        if self.style.is_some() && next_head.is_finite() {
            match self
                .pending
                .iter()
                .find(|p| p.lane == lane && (p.cross - next_head).abs() < SAME_NOTE_MS)
            {
                Some(p) if p.draw.miss => next_press = f64::INFINITY,
                Some(p) => {
                    next_press =
                        (next_press + p.draw.error_ms).max(press + MIN_HELD_MS + c.min_gap_ms)
                }
                None => next_press = next_press.max(press + MIN_HELD_MS + c.min_gap_ms),
            }
        }
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
            let draw = if self.style.is_some() {
                self.draw_for(tr, t.id, t.lane, cross)
            } else {
                NoteDraw::default()
            };
            if draw.miss {
                if cross + c.offset_ms - now_ms < horizon {
                    self.style_misses += 1;
                    self.commits.push(Commit {
                        id: t.id,
                        lane: t.lane,
                        cross,
                        press: f64::NAN,
                        release: f64::NAN,
                        rel_err: 0.0,
                    });
                }
                continue;
            }
            let mut press = cross + c.offset_ms + draw.error_ms;
            if self.style.is_some() {
                press = press.max(self.lane_floor(t.lane, cross));
            }
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
                        rel_err: 0.0,
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
            let release = self.release_for(tr, t.lane, cross, press, now_ms, draw.release_error_ms);
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
                rel_err: draw.release_error_ms,
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
            let mut rel = self.release_for(tr, k.lane, k.cross, k.press, now_ms, k.rel_err);
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
        self.pending.retain(|p| p.cross >= now_ms - 2000.0);
        self.commits.retain(|k| {
            !(k.release < now_ms - 2000.0 || (k.press.is_nan() && k.cross < now_ms - 2000.0))
        });
    }
}
