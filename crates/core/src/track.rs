//! Tracking and scroll velocity (spec 04): note observations -> tracks with predicted crossings.

use crate::detect::{Layout, NoteObs, ObsKind};
use crate::time::Nanos;

const HIST: usize = 16;

#[derive(Clone, Debug)]
pub struct Track {
    pub id: u32,
    pub lane: u8,
    /// Trail-end track (hold end) rather than a head.
    pub tail: bool,
    /// Last `HIST` observations: (ms since the tracker's origin, y).
    obs: [(f64, f32); HIST],
    pub n: u32,
    first: (f64, f32),
    pub confirmed: bool,
    pub done: bool,
    /// Last time it was observed, or known to be hidden under other heads of its lane.
    seen: f64,
}

impl Track {
    fn hist(&self) -> &[(f64, f32)] {
        &self.obs[..(self.n as usize).min(HIST)]
    }

    pub fn last(&self) -> (f64, f32) {
        self.obs[(self.n as usize - 1) % HIST]
    }

    fn push(&mut self, t: f64, y: f32) {
        self.obs[self.n as usize % HIST] = (t, y);
        self.n += 1;
        self.seen = t;
    }
}

/// Unconfirmed tracks with no velocity yet may move this far (in head lengths) per frame.
const FREE_GATE: f32 = 1.5;

pub struct Tracker {
    ry: [f32; 4],
    approach: f32,
    radius: f32,
    gate: [f32; 4],
    origin: Option<Nanos>,
    tracks: Vec<Track>,
    next_id: u32,
    /// Scroll velocity, px per ms (signed, y per ms).
    v: Option<f64>,
    slopes: Vec<f64>,
    order: Vec<usize>,
    /// Drop a track after this long without an observation.
    pub max_gap_ms: f64,
    pub scroll_changes: u32,
    /// Per lane: last time a trail touched the receptor zone or ran off the strip, and whether a
    /// trail runs off the strip in the latest frame.
    trail_seen: [f64; 4],
    trail_cut: [bool; 4],
}

fn median(v: &mut [f64]) -> f64 {
    let m = v.len() / 2;
    *v.select_nth_unstable_by(m, |a, b| a.total_cmp(b)).1
}

impl Tracker {
    pub fn new(layout: &Layout, head_len: [f32; 4]) -> Self {
        Self {
            ry: layout.receptors.map(|r| r[1]),
            approach: layout.approach() as f32,
            radius: layout.radius(),
            gate: head_len,
            origin: None,
            tracks: Vec::with_capacity(256),
            next_id: 1,
            v: None,
            slopes: Vec::with_capacity(4096),
            order: Vec::with_capacity(64),
            max_gap_ms: 120.0,
            scroll_changes: 0,
            trail_seen: [f64::NEG_INFINITY; 4],
            trail_cut: [false; 4],
        }
    }

    /// A trail runs off the far end of `lane`'s strip in the latest frame.
    pub fn trail_cut(&self, lane: u8) -> bool {
        self.trail_cut[lane as usize]
    }

    /// Last time (tracker ms) a trail was attached to the receptor or ran off the strip.
    pub fn trail_seen(&self, lane: u8) -> f64 {
        self.trail_seen[lane as usize]
    }

    pub fn velocity(&self) -> Option<f64> {
        self.v
    }

    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    /// Ms since the tracker's origin, the time base of `Track` observations.
    pub fn t(&self, pts: Nanos) -> f64 {
        self.origin.map_or(0.0, |o| pts.ms_since(o))
    }

    pub fn to_nanos(&self, t: f64) -> Nanos {
        self.origin.unwrap_or_default().add_ms(t)
    }

    /// Distance toward the far end of the strip (positive before the crossing).
    fn s(&self, lane: usize, y: f32) -> f32 {
        (y - self.ry[lane]) * self.approach
    }

    fn predict(&self, tr: &Track, t: f64) -> f32 {
        let (lt, ly) = tr.last();
        let v = match self.v {
            Some(v) => v,
            None if tr.n >= 2 => {
                let (pt, py) = tr.obs[(tr.n as usize - 2) % HIST];
                if lt > pt {
                    (ly - py) as f64 / (lt - pt)
                } else {
                    0.0
                }
            }
            None => 0.0,
        };
        ly + (v * (t - lt)) as f32
    }

    /// Robust per-track intercept: median of `y - v t`.
    /// Per-track intercept of `y - v t`: mean with the lowest and highest quarter dropped
    /// (pts jitter is roughly uniform, where a trimmed mean beats the median).
    fn intercept(&self, tr: &Track, v: f64, buf: &mut [f64; HIST]) -> f64 {
        let h = tr.hist();
        for (i, &(t, y)) in h.iter().enumerate() {
            buf[i] = y as f64 - v * t;
        }
        let b = &mut buf[..h.len()];
        b.sort_by(|x, y| x.total_cmp(y));
        let k = b.len() / 4;
        let mid = &b[k..b.len() - k];
        mid.iter().sum::<f64>() / mid.len() as f64
    }

    /// Predicted time the track reaches its receptor, in tracker ms.
    pub fn cross_ms(&self, tr: &Track) -> Option<f64> {
        let v = self.v?;
        if v.abs() < 1e-3 {
            return None;
        }
        let b = self.intercept(tr, v, &mut [0.0; HIST]);
        Some((self.ry[tr.lane as usize] as f64 - b) / v)
    }

    pub fn update(&mut self, pts: Nanos, obs: &[NoteObs]) {
        let origin = *self.origin.get_or_insert(pts);
        let t = pts.ms_since(origin);
        self.trail_cut = [false; 4];
        for o in obs {
            let l = o.lane as usize;
            if matches!(o.kind, ObsKind::TrailStart | ObsKind::TrailCut) {
                self.trail_seen[l] = t;
            }
            if o.kind == ObsKind::TrailCut {
                self.trail_cut[l] = true;
            }
        }
        for lane in 0..4u8 {
            for tail in [false, true] {
                self.associate(lane, tail, t, obs);
            }
        }
        self.fit_velocity(t);
        // Overlapping heads (slow jacks) only show their two ends; a confirmed head predicted
        // between two head observations of its lane that close together is hidden, not gone.
        for i in 0..self.tracks.len() {
            let tr = &self.tracks[i];
            if !tr.confirmed || tr.tail || tr.seen >= t {
                continue;
            }
            let (l, y) = (tr.lane, self.predict(tr, t));
            let span = 2.5 * self.gate[l as usize];
            let heads = || {
                obs.iter()
                    .filter(|o| o.lane == l && o.kind == ObsKind::Head)
            };
            let hidden = heads().any(|a| heads().any(|b| a.y < y && y < b.y && b.y - a.y < span));
            if hidden {
                self.tracks[i].seen = t;
            }
        }
        // End tracks: unseen too long, past the receptor by more than a radius, or (confirmed)
        // 20 ms past their crossing. After the crossing a lit receptor sits exactly where the
        // head was and would otherwise keep the track alive as a stationary note.
        let (radius, gap) = (self.radius, self.max_gap_ms);
        for i in 0..self.tracks.len() {
            let tr = &self.tracks[i];
            let past = self.s(tr.lane as usize, self.predict(tr, t)) < -radius;
            let crossed = tr.confirmed && self.cross_ms(tr).is_some_and(|c| t > c + 20.0);
            if t - tr.seen > gap || past || crossed {
                self.tracks[i].done = true;
            }
        }
        self.tracks.retain(|tr| !tr.done);
    }

    fn associate(&mut self, lane: u8, tail: bool, t: f64, obs: &[NoteObs]) {
        let li = lane as usize;
        let want = if tail {
            ObsKind::TrailEnd
        } else {
            ObsKind::Head
        };
        // Observations nearest the receptor first; tracks in the same order (no overtaking).
        let mut ys: [f32; 32] = [0.0; 32];
        let mut k = 0;
        for o in obs
            .iter()
            .filter(|o| o.lane == lane && o.kind == want)
            .take(32)
        {
            ys[k] = o.y;
            k += 1;
        }
        let ys = &mut ys[..k];
        ys.sort_by(|a, b| self.s(li, *a).total_cmp(&self.s(li, *b)));
        self.order.clear();
        self.order.extend(
            (0..self.tracks.len())
                .filter(|&i| self.tracks[i].lane == lane && self.tracks[i].tail == tail),
        );
        let mut order = std::mem::take(&mut self.order);
        order.sort_by(|&a, &b| {
            let pa = self.s(li, self.predict(&self.tracks[a], t));
            let pb = self.s(li, self.predict(&self.tracks[b], t));
            pa.total_cmp(&pb)
        });
        let mut from = 0;
        for &y in ys.iter() {
            let mut best: Option<(usize, f32)> = None;
            for (j, &ti) in order.iter().enumerate().skip(from) {
                let tr = &self.tracks[ti];
                let d = y - self.predict(tr, t);
                let gate = if let Some(v) = self.v {
                    // Also at most 15 ms of travel, so the next note of a 50 ms jack (which
                    // overlaps this one at low speed) is never taken for this one.
                    (0.35 * self.gate[li]).min(15.0 * v.abs() as f32).max(3.0)
                } else if tr.n >= 2 {
                    (0.35 * self.gate[li]).max(6.0)
                } else {
                    FREE_GATE * self.gate[li]
                };
                // Without a velocity, a young track may only move toward the receptor.
                let ok = if self.v.is_none() && tr.n < 2 {
                    let step = -d * self.approach;
                    (-3.0..=gate).contains(&step)
                } else {
                    d.abs() <= gate
                };
                if ok && best.is_none_or(|b| d.abs() < b.1) {
                    best = Some((j, d.abs()));
                }
            }
            match best {
                Some((j, _)) => {
                    from = j + 1;
                    self.tracks[order[j]].push(t, y);
                }
                None => {
                    let id = self.next_id;
                    self.next_id += 1;
                    let mut tr = Track {
                        id,
                        lane,
                        tail,
                        obs: [(0.0, 0.0); HIST],
                        n: 0,
                        first: (t, y),
                        confirmed: false,
                        done: false,
                        seen: t,
                    };
                    tr.push(t, y);
                    self.tracks.push(tr);
                }
            }
        }
        // Confirm: 3+ observations, moving toward the receptor (a lit receptor stands still).
        for &ti in &order {
            let tr = &self.tracks[ti];
            if tr.confirmed || tr.n < 3 {
                continue;
            }
            let (lt, ly) = tr.last();
            let moved = (self.s(li, tr.first.1) - self.s(li, ly)) as f64;
            let rate = moved / (lt - tr.first.0).max(1e-3);
            let ok = match self.v {
                // 15 ms of history keeps 1 ms pts jitter under ~13% of the rate.
                Some(v) => lt - tr.first.0 >= 15.0 && (rate - v.abs()).abs() <= 0.3 * v.abs(),
                None => moved >= 2.0 * (tr.n - 1) as f64 && rate > 0.05,
            };
            if ok {
                self.tracks[ti].confirmed = true;
            }
        }
        self.order = order;
    }

    /// Theil-Sen over within-track pairs of confirmed tracks; refit on the last 150 ms when
    /// the residuals say the speed changed.
    fn fit_velocity(&mut self, t: f64) {
        let Some(v500) = self.slopes_since(t - 500.0) else {
            return;
        };
        let mut v = v500;
        if self.v.is_some() && self.residual(v500) > 2.0 {
            // Real footage has a few px of residual noise; only a real change in speed counts.
            if let Some(v150) = self.slopes_since(t - 150.0)
                && (v150 - v500).abs() > 0.05 * v500.abs()
            {
                v = v150;
                self.scroll_changes += 1;
            }
        }
        // Notes always move toward the receptor.
        if v * self.approach as f64 >= 0.0 {
            return;
        }
        self.v = Some(v);
    }

    fn slopes_since(&mut self, t0: f64) -> Option<f64> {
        self.slopes.clear();
        // Observations inside the receptor zone are left out: glow and splashes sit there.
        let (ry, a, r) = (self.ry, self.approach, self.radius);
        let outside = |lane: u8, y: f32| (y - ry[lane as usize]) * a > r;
        // The first speed comes from heads only: a grey trail end can be HUD text fading in or
        // out (the countdown's key labels under the receptors), which crept at 0.15 px/ms and
        // set a speed that real notes then never matched (spec 13, 6.2).
        let tails = self.v.is_some();
        for tr in self
            .tracks
            .iter()
            .filter(|tr| tr.confirmed && (tails || !tr.tail))
        {
            let h = tr.hist();
            for (i, a) in h.iter().enumerate() {
                for b in &h[i + 1..] {
                    let (p, q) = if a.0 < b.0 { (a, b) } else { (b, a) };
                    if p.0 >= t0
                        && q.0 - p.0 >= 10.0
                        && outside(tr.lane, p.1)
                        && outside(tr.lane, q.1)
                        && self.slopes.len() < self.slopes.capacity()
                    {
                        self.slopes.push((q.1 - p.1) as f64 / (q.0 - p.0));
                    }
                }
            }
        }
        (self.slopes.len() >= 6).then(|| median(&mut self.slopes))
    }

    /// Median |residual| of the newest observation of each confirmed track under `v`.
    fn residual(&mut self, v: f64) -> f64 {
        let mut buf = [0.0; HIST];
        let mut res = [0.0f64; 64];
        let mut k = 0;
        for tr in self.tracks.iter().filter(|tr| tr.confirmed && tr.n >= 4) {
            if k == res.len() {
                break;
            }
            let (lt, ly) = tr.last();
            // Intercept from the older observations only.
            let h = tr.hist();
            let mut m = 0;
            for &(ot, oy) in h.iter().filter(|o| o.0 < lt) {
                buf[m] = oy as f64 - v * ot;
                m += 1;
            }
            if m == 0 {
                continue;
            }
            let b = median(&mut buf[..m]);
            res[k] = (ly as f64 - (b + v * lt)).abs();
            k += 1;
        }
        if k < 3 { 0.0 } else { median(&mut res[..k]) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Scroll;

    #[test]
    fn countdown_text_does_not_set_the_speed() {
        // Upscroll, receptors at y 200. During the countdown grey key labels below the
        // receptors move with the stage's zoom-in, then fade (read as trail ends); then notes
        // come up at 2 px/ms. The speed must come from the notes.
        let layout = Layout {
            receptors: [
                [100.0, 200.0],
                [200.0, 200.0],
                [300.0, 200.0],
                [400.0, 200.0],
            ],
            size: 140.0,
            run_len: [130.0; 4],
            run_shape: [[-65.0, 0.0, 65.0]; 4],
            scroll: Scroll::Up,
            stream: [1000, 1440],
        };
        let mut tr = Tracker::new(&layout, [130.0; 4]);
        let obs = |lane: u8, y: f32, kind| NoteObs {
            lane,
            y,
            kind,
            pts: Nanos(0),
        };
        let mut t: f64 = 0.0;
        while t < 1500.0 {
            let o: Vec<NoteObs> = (0..4)
                // the stage zooms in (0.5 px/ms for 150 ms), then the labels fade and creep
                .map(|l| {
                    let y = 430.0 - 0.5 * t.min(150.0) as f32 - 0.02 * (t - 150.0).max(0.0) as f32;
                    obs(l, y, ObsKind::TrailEnd)
                })
                .collect();
            tr.update(Nanos::from_ms(t), &o);
            t += 6.0;
        }
        assert_eq!(tr.velocity(), None, "labels gave a speed");
        let start = t;
        while t < start + 600.0 {
            let y = 1300.0 - 2.0 * (t - start) as f32;
            tr.update(
                Nanos::from_ms(t),
                &[obs(0, y, ObsKind::Head), obs(2, y + 300.0, ObsKind::Head)],
            );
            t += 6.0;
        }
        let v = tr.velocity().expect("notes give a speed");
        assert!((v + 2.0).abs() < 0.05, "{v}");
    }
}
