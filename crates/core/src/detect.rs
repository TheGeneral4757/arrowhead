//! Strip classifier and runs: frames -> `NoteObs` (spec 03).

use serde::{Deserialize, Serialize};

use crate::config::Scroll;
use crate::frame::{FrameView, Rect};
use crate::time::Nanos;

pub mod calib;
#[cfg(test)]
pub(crate) mod synth_tests;

/// Opponent colour space `(C1, C2, Y)`: `C1 = r - b`, `C2 = g - (r + b) / 2`, `Y = (r + 2g + b) / 4`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Opp(pub i16, pub i16, pub i16);

impl Opp {
    #[inline(always)]
    pub fn from_rgb(r: u8, g: u8, b: u8) -> Self {
        let (r, g, b) = (r as i16, g as i16, b as i16);
        Opp(r - b, g - ((r + b) >> 1), (r + 2 * g + b) >> 2)
    }

    /// L1 distance.
    #[inline(always)]
    pub fn dist(self, o: Opp) -> i32 {
        (self.0 as i32 - o.0 as i32).abs()
            + (self.1 as i32 - o.1 as i32).abs()
            + (self.2 as i32 - o.2 as i32).abs()
    }

    #[inline(always)]
    pub fn chroma(self) -> i32 {
        (self.0 as i32).abs() + (self.1 as i32).abs()
    }
}

/// Profile `[detect]`; defaults from spec 03.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DetectConfig {
    pub strip_width_frac: f32,
    pub approach_len_frac: f32,
    pub head_fill: f32,
    pub trail_fill: f32,
    pub head_tol_min: i32,
    pub head_tol_max: i32,
    pub trail_tol_min: i32,
    pub trail_tol_max: i32,
    pub bg_tol: i32,
    /// Min `|C1| + |C2|` for the "saturated" half of the learning rule.
    pub learn_chroma: i32,
    pub receptor_ncc_min: f32,
    /// HUD glyph match: mean absolute difference (0..1) above which a read is rejected.
    pub glyph_max_sad: f32,
}

impl Default for DetectConfig {
    fn default() -> Self {
        Self {
            strip_width_frac: 0.35,
            approach_len_frac: 0.70,
            head_fill: 0.4,
            trail_fill: 0.4,
            head_tol_min: 12,
            head_tol_max: 60,
            trail_tol_min: 12,
            trail_tol_max: 40,
            bg_tol: 18,
            learn_chroma: 40,
            receptor_ncc_min: 0.6,
            glyph_max_sad: 0.25,
        }
    }
}

/// Learned per profile (spec 03, colour learning).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Colours {
    pub heads: [Opp; 4],
    pub head_tol: [i32; 4],
    /// `None` until learned; no trails are reported until then.
    pub trail: Option<Opp>,
    pub trail_tol: i32,
    /// Median length of one head's run along the strip per lane, px.
    pub head_len: [f32; 4],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    /// Receptor sprite centres (middle of the bounding box), stream px.
    pub receptors: [[f32; 2]; 4],
    /// Receptor (and head) diameter, px.
    pub size: f32,
    /// Length of each receptor's run of filled rows in its strip. A head has the same shape,
    /// so its run should match (arrows pointing sideways are shorter than `size`).
    pub run_len: [f32; 4],
    /// That run's (start, fill-weighted centroid, end) row minus the sprite centre y. Arrows
    /// are not symmetric along the strip, so a head's centre is recovered from these.
    pub run_shape: [[f32; 3]; 4],
    pub scroll: Scroll,
    pub stream: [u32; 2],
}

impl Layout {
    pub fn pitch(&self) -> f32 {
        (self.receptors[3][0] - self.receptors[0][0]) / 3.0
    }

    pub fn radius(&self) -> f32 {
        self.size / 2.0
    }

    /// +1 when notes come from larger y (upscroll), -1 for downscroll.
    pub fn approach(&self) -> i32 {
        match self.scroll {
            Scroll::Up => 1,
            Scroll::Down => -1,
        }
    }

    pub fn strip_width(&self, cfg: &DetectConfig) -> u32 {
        (cfg.strip_width_frac * self.pitch()).max(6.0).round() as u32
    }

    /// Strip for `lane`: one radius past the receptor, then `approach_len` back along the approach.
    pub fn strip(&self, lane: usize, cfg: &DetectConfig) -> Rect {
        let [cx, cy] = self.receptors[lane];
        let w = self.strip_width(cfg);
        let len = cfg.approach_len_frac * self.stream[1] as f32;
        let r = self.radius();
        let (y0, y1) = match self.scroll {
            Scroll::Up => (cy - r, cy + len),
            Scroll::Down => (cy - len, cy + r),
        };
        let y0 = y0.max(0.0).floor() as i32;
        let y1 = y1.min(self.stream[1] as f32).ceil() as i32;
        let x0 = (cx - w as f32 / 2.0).round() as i32;
        Rect::new(x0, y0, w, (y1 - y0).max(0) as u32)
    }

    /// Capture region: bounding box of the 4 strips plus a 1-radius margin, clipped to the stream.
    pub fn region(&self, cfg: &DetectConfig) -> Rect {
        let m = self.radius().ceil() as i32;
        let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for l in 0..4 {
            let s = self.strip(l, cfg);
            x0 = x0.min(s.x - m);
            y0 = y0.min(s.y - m);
            x1 = x1.max(s.x + s.w as i32 + m);
            y1 = y1.max(s.y + s.h as i32 + m);
        }
        let (x0, y0) = (x0.max(0), y0.max(0));
        let x1 = x1.min(self.stream[0] as i32);
        let y1 = y1.min(self.stream[1] as i32);
        Rect::new(x0, y0, (x1 - x0) as u32, (y1 - y0) as u32)
    }

    /// Stream rows `[a, b)` of the receptor zone (within one radius of the centre).
    pub fn receptor_zone(&self, lane: usize) -> (i32, i32) {
        let cy = self.receptors[lane][1];
        let r = self.radius();
        ((cy - r).floor() as i32, (cy + r).ceil() as i32 + 1)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ObsKind {
    Head,
    /// Near end of a trail with no head (hold in progress, head already at the receptor).
    TrailStart,
    /// Far end of a trail.
    TrailEnd,
    /// The trail runs off the far end of the strip: a hold whose end is not visible yet.
    TrailCut,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct NoteObs {
    pub lane: u8,
    /// Stream y, sub-pixel.
    pub y: f32,
    pub kind: ObsKind,
    pub pts: Nanos,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Run {
    pub(crate) a: i32,
    pub(crate) b: i32,
    pub(crate) w: f32,
    pub(crate) wy: f32,
}

/// Contiguous rows with `fill[i] >= thr`; rows are stream y `y0 + i`.
fn runs(fill: &[u16], thr: u16, y0: i32, out: &mut Vec<Run>) {
    out.clear();
    let mut cur: Option<Run> = None;
    for (i, &f) in fill.iter().enumerate() {
        let y = y0 + i as i32;
        if f >= thr && f > 0 {
            let r = cur.get_or_insert(Run {
                a: y,
                b: y,
                w: 0.0,
                wy: 0.0,
            });
            r.b = y;
            r.w += f as f32;
            r.wy += f as f32 * y as f32;
        } else if let Some(r) = cur.take() {
            out.push(r);
        }
    }
    out.extend(cur);
}

struct Lane {
    strip: Rect,
    zone: (i32, i32),
    /// Background luma per strip pixel, fixed point x16.
    bg: Vec<u16>,
    head: Vec<u16>,
    trail: Vec<u16>,
}

/// Per-frame detector. Allocates only in `new`; `detect` reuses its buffers.
pub struct Detector {
    pub layout: Layout,
    pub cfg: DetectConfig,
    pub colours: Colours,
    lanes: Vec<Lane>,
    heads: Vec<Run>,
    trails: Vec<Run>,
    primed: bool,
    tick: u32,
}

impl Detector {
    pub fn new(layout: Layout, cfg: DetectConfig, colours: Colours) -> Self {
        let lanes = (0..4)
            .map(|l| {
                let strip = layout.strip(l, &cfg);
                let n = (strip.w * strip.h) as usize;
                Lane {
                    strip,
                    zone: layout.receptor_zone(l),
                    bg: vec![0; n],
                    head: vec![0; strip.h as usize],
                    trail: vec![0; strip.h as usize],
                }
            })
            .collect();
        Self {
            layout,
            cfg,
            colours,
            lanes,
            heads: Vec::with_capacity(64),
            trails: Vec::with_capacity(64),
            primed: false,
            tick: 0,
        }
    }

    /// Background luma for `lane`'s strip, row-major `strip.w` wide (from calibration).
    pub fn set_background(&mut self, lane: usize, luma: &[u8]) {
        for (b, &l) in self.lanes[lane].bg.iter_mut().zip(luma) {
            *b = l as u16 * 16;
        }
        self.primed = true;
    }

    /// The background as adapted so far, in `set_background`'s form.
    pub fn background(&self, lane: usize) -> Vec<u8> {
        self.lanes[lane]
            .bg
            .iter()
            .map(|&b| (b / 16) as u8)
            .collect()
    }

    pub fn strip(&self, lane: usize) -> Rect {
        self.lanes[lane].strip
    }

    /// Observations for one frame into `out` (cleared first), ordered by lane.
    pub fn detect(&mut self, f: &FrameView, out: &mut Vec<NoteObs>) {
        out.clear();
        let prime = !self.primed;
        self.primed = true;
        // The background drifts slowly; updating it every 4th frame is enough and halves the cost.
        self.tick = self.tick.wrapping_add(1);
        let update_bg = self.tick.is_multiple_of(4);
        let bg_tol = self.cfg.bg_tol * 16;
        let approach = self.layout.approach();
        for li in 0..4 {
            let lane = &mut self.lanes[li];
            lane.head.fill(0);
            lane.trail.fill(0);
            let Some(r) = lane.strip.intersect(&f.region) else {
                continue;
            };
            let hl = self.colours.head_len[li].max(2.0);
            let sh = self.layout.run_shape[li].map(|v| v * hl / self.layout.run_len[li].max(1.0));
            let hc = self.colours.heads[li];
            let ht = self.colours.head_tol[li];
            let (trail_c, tt) = (self.colours.trail, self.colours.trail_tol);
            let sw = lane.strip.w as usize;
            let dx = (r.x - lane.strip.x) as usize;
            for y in r.y..r.y + r.h as i32 {
                let ri = (y - lane.strip.y) as usize;
                let off = (y - f.region.y) as usize * f.stride + (r.x - f.region.x) as usize * 4;
                let px = &f.data[off..off + r.w as usize * 4];
                let bg = &mut lane.bg[ri * sw + dx..ri * sw + dx + r.w as usize];
                let in_zone = y >= lane.zone.0 && y < lane.zone.1;
                let (mut nh, mut nt) = (0u16, 0u16);
                for (p, b) in px.chunks_exact(4).zip(bg.iter_mut()) {
                    let o = Opp::from_rgb(p[2], p[1], p[0]);
                    let y16 = o.2 as i32 * 16;
                    if prime {
                        *b = y16 as u16;
                    }
                    if o.dist(hc) <= ht {
                        nh += 1;
                        continue;
                    }
                    // Unlearned trail: no trail detection (a grey rule here would match a black
                    // background whenever the background model is off).
                    let is_trail = !in_zone
                        && trail_c.is_some_and(|t| o.dist(t) <= tt)
                        && (y16 - *b as i32).abs() > bg_tol;
                    if is_trail {
                        nt += 1;
                    } else if update_bg {
                        *b = (*b as i32 + ((y16 - *b as i32) >> 3)) as u16;
                    }
                }
                lane.head[ri] = nh;
                lane.trail[ri] = nt;
            }

            let w = sw as f32;
            runs(
                &lane.head,
                (self.cfg.head_fill * w).ceil() as u16,
                lane.strip.y,
                &mut self.heads,
            );
            runs(
                &lane.trail,
                (self.cfg.trail_fill * w).ceil() as u16,
                lane.strip.y,
                &mut self.trails,
            );
            let pts = f.pts;
            let lane_u8 = li as u8;
            let mut push = |y: f32, kind| {
                out.push(NoteObs {
                    lane: lane_u8,
                    y,
                    kind,
                    pts,
                })
            };

            let (top, bottom) = (lane.strip.y, lane.strip.y + lane.strip.h as i32 - 1);
            // Runs are in y order; `prev_far` is the far end (approach side) of the previous run
            // nearer the receptor, so a run starting right behind it is a note partly covered
            // by that one (the near note's outline cuts across it).
            for h in &self.heads {
                let len = (h.b - h.a + 1) as f32;
                if len < 0.4 * hl {
                    continue;
                }
                // Cut by the far end of the strip: only the near end is real.
                let cut = if approach > 0 {
                    h.b >= bottom
                } else {
                    h.a <= top
                };
                let (from_a, from_b) = (h.a as f32 - sh[0], h.b as f32 - sh[2]);
                if cut {
                    if len >= 0.9 * hl {
                        push(if approach > 0 { from_a } else { from_b }, ObsKind::Head);
                    }
                } else if len > 1.2 * hl {
                    // Overlapping heads (jacks at low speed): only the two ends are exposed.
                    // shortcut: middle notes of 3+ overlapping heads are seen only once the
                    // notes in front are hit; splitting on note outlines needs real footage.
                    push(from_a, ObsKind::Head);
                    push(from_b, ObsKind::Head);
                } else {
                    push(h.wy / h.w - sh[1], ObsKind::Head);
                }
            }

            let gap = (0.5 * hl) as i32 + 1;
            for t in &self.trails {
                // near = end toward the receptor, far = end the notes come from.
                let (near, far) = if approach > 0 { (t.a, t.b) } else { (t.b, t.a) };
                let behind_head = self.heads.iter().any(|h| {
                    let hfar = if approach > 0 { h.b } else { h.a };
                    (near - hfar) * approach >= -1 && (near - hfar) * approach <= gap
                });
                if !behind_head {
                    let zone_edge = if approach > 0 {
                        lane.zone.1
                    } else {
                        lane.zone.0 - 1
                    };
                    if (near - zone_edge).abs() <= gap {
                        push(near as f32, ObsKind::TrailStart);
                    }
                }
                let cut = if approach > 0 {
                    far >= bottom
                } else {
                    far <= top
                };
                push(
                    far as f32,
                    if cut {
                        ObsKind::TrailCut
                    } else {
                        ObsKind::TrailEnd
                    },
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opp_basics() {
        let grey = Opp::from_rgb(85, 85, 85);
        assert_eq!(grey, Opp(0, 0, 85));
        assert_eq!(Opp::from_rgb(255, 0, 0), Opp(255, -127, 63));
        assert_eq!(grey.dist(Opp(0, 0, 80)), 5);
    }

    #[test]
    fn runs_split_on_gaps() {
        let mut out = Vec::new();
        runs(&[0, 5, 5, 0, 0, 5, 1, 5], 3, 100, &mut out);
        let ab: Vec<_> = out.iter().map(|r| (r.a, r.b)).collect();
        assert_eq!(ab, vec![(101, 102), (105, 105), (107, 107)]);
        assert_eq!(out[0].wy / out[0].w, 101.5);
    }

    #[test]
    fn strips_follow_scroll() {
        let mut l = Layout {
            receptors: [
                [100.0, 100.0],
                [200.0, 100.0],
                [300.0, 100.0],
                [400.0, 100.0],
            ],
            size: 60.0,
            run_len: [60.0; 4],
            run_shape: [[-29.5, 0.0, 29.5]; 4],
            scroll: Scroll::Up,
            stream: [1000, 1000],
        };
        let cfg = DetectConfig::default();
        let s = l.strip(0, &cfg);
        assert_eq!((s.x, s.y, s.w, s.h), (83, 70, 35, 730));
        l.scroll = Scroll::Down;
        let s = l.strip(0, &cfg);
        assert_eq!((s.y, s.h), (0, 130));
        assert!(l.region(&cfg).w >= 335);
    }
}
