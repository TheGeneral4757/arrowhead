//! Calibration (spec 03): receptor finding, side, scroll guess, colour learning,
//! receptor presence. Runs off the hot path; allocates freely.

use crate::config::{Scroll, Side};
use crate::frame::{FrameView, Rect};

use super::{Colours, DetectConfig, Layout, Opp};

#[derive(Clone, Copy, Debug)]
pub struct Blob {
    /// Bbox centre, stream px.
    pub cx: f32,
    pub cy: f32,
    /// Max bbox side, stream px.
    pub diam: f32,
    /// sqrt(pixel count), stream px; equal for rotated copies of one shape.
    pub size: f32,
    /// Brighter than its surround.
    pub bright: bool,
}

#[derive(Clone, Debug)]
pub struct Group {
    pub blobs: [Blob; 4],
    pub cx: f32,
    pub cy: f32,
    pub diam: f32,
    pub pitch: f32,
}

fn luma(p: &[u8]) -> u16 {
    (p[2] as u16 + 2 * p[1] as u16 + p[0] as u16) / 4
}

fn median_u8(v: &mut [u8]) -> u8 {
    let m = v.len() / 2;
    *v.select_nth_unstable(m).1
}

/// Per-pixel median luma of `r` (stream coords, inside every frame's region), downscaled by `ds`.
fn median_luma(frames: &[FrameView], r: Rect, ds: u32) -> (Vec<u8>, usize, usize) {
    let (w, h) = ((r.w / ds) as usize, (r.h / ds) as usize);
    let mut out = vec![0u8; w * h];
    let mut tmp = vec![0u8; frames.len()];
    for y in 0..h {
        for x in 0..w {
            for (k, f) in frames.iter().enumerate() {
                let mut s = 0u32;
                for j in 0..ds {
                    let sy = r.y as usize + y * ds as usize + j as usize - f.region.y as usize;
                    for i in 0..ds {
                        let sx = r.x as usize + x * ds as usize + i as usize - f.region.x as usize;
                        s += luma(&f.data[sy * f.stride + sx * 4..]) as u32;
                    }
                }
                tmp[k] = (s / (ds * ds)) as u8;
            }
            out[y * w + x] = median_u8(&mut tmp);
        }
    }
    (out, w, h)
}

/// Blobs that contrast with their surround in the median of `frames` (moving things drop out).
pub fn find_blobs(frames: &[FrameView], stream_h: u32) -> Vec<Blob> {
    let region = frames[0].region;
    let ds = 2u32;
    let (l, w, h) = median_luma(frames, region, ds);
    // Integral image for the local mean.
    let mut ii = vec![0u64; (w + 1) * (h + 1)];
    for y in 0..h {
        let mut row = 0u64;
        for x in 0..w {
            row += l[y * w + x] as u64;
            ii[(y + 1) * (w + 1) + x + 1] = ii[y * (w + 1) + x + 1] + row;
        }
    }
    let rad = (stream_h as usize / ds as usize / 12).max(4);
    // mask: 0 none, 1 bright, 2 dark
    let mut mask = vec![0u8; w * h];
    for y in 0..h {
        let (y0, y1) = (y.saturating_sub(rad), (y + rad + 1).min(h));
        for x in 0..w {
            let (x0, x1) = (x.saturating_sub(rad), (x + rad + 1).min(w));
            let s = ii[y1 * (w + 1) + x1] + ii[y0 * (w + 1) + x0]
                - ii[y0 * (w + 1) + x1]
                - ii[y1 * (w + 1) + x0];
            let mean = s as i32 / ((y1 - y0) * (x1 - x0)) as i32;
            let d = l[y * w + x] as i32 - mean;
            mask[y * w + x] = if d > 30 {
                1
            } else if d < -30 {
                2
            } else {
                0
            };
        }
    }
    let hh = (stream_h / ds) as f32;
    let (amin, amax) = (0.001 * hh * hh, 0.05 * hh * hh);
    let mut seen = vec![false; w * h];
    let mut stack = Vec::new();
    let mut blobs = Vec::new();
    for start in 0..w * h {
        if mask[start] == 0 || seen[start] {
            continue;
        }
        let m = mask[start];
        seen[start] = true;
        stack.push(start);
        let (mut n, mut x0, mut y0, mut x1, mut y1) = (0usize, w, h, 0, 0);
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            n += 1;
            x0 = x0.min(x);
            x1 = x1.max(x);
            y0 = y0.min(y);
            y1 = y1.max(y);
            let mut go = |j: usize| {
                if mask[j] == m && !seen[j] {
                    seen[j] = true;
                    stack.push(j);
                }
            };
            if x > 0 {
                go(i - 1);
            }
            if x + 1 < w {
                go(i + 1);
            }
            if y > 0 {
                go(i - w);
            }
            if y + 1 < h {
                go(i + w);
            }
        }
        let (bw, bh) = ((x1 - x0 + 1) as f32, (y1 - y0 + 1) as f32);
        let area = bw * bh;
        if area < amin || area > amax || (n as f32) < 0.2 * area || bw.max(bh) > 2.0 * bw.min(bh) {
            continue;
        }
        let s = ds as f32;
        blobs.push(Blob {
            cx: region.x as f32 + (x0 + x1 + 1) as f32 * s / 2.0,
            cy: region.y as f32 + (y0 + y1 + 1) as f32 * s / 2.0,
            diam: bw.max(bh) * s,
            size: (n as f32).sqrt() * s,
            bright: m == 1,
        });
    }
    blobs
}

/// Rows of 4 alike, equally spaced blobs (spec 03 receptor finding, steps 1-2).
pub fn find_groups(blobs: &[Blob], stream_h: u32) -> Vec<Group> {
    let mut b = blobs.to_vec();
    b.sort_by(|p, q| p.cx.total_cmp(&q.cx));
    let ytol = 0.02 * stream_h as f32;
    let alike = |p: &Blob, q: &Blob| {
        (p.cy - q.cy).abs() <= ytol
            && (p.size - q.size).abs() <= 0.15 * p.size.max(q.size)
            && p.bright == q.bright
    };
    let mut cands: Vec<(f32, [usize; 4])> = Vec::new();
    for i in 0..b.len() {
        for j in i + 1..b.len() {
            if !alike(&b[i], &b[j]) {
                continue;
            }
            let p = b[j].cx - b[i].cx;
            let d = b[i].diam.max(b[j].diam);
            if p < 0.9 * d || p > 3.0 * d {
                continue;
            }
            let near = |x: f32| {
                (0..b.len())
                    .filter(|&k| alike(&b[i], &b[k]) && (b[k].cx - x).abs() <= 0.08 * p)
                    .min_by(|&u, &v| (b[u].cx - x).abs().total_cmp(&(b[v].cx - x).abs()))
            };
            let (Some(k), Some(l)) = (near(b[i].cx + 2.0 * p), near(b[i].cx + 3.0 * p)) else {
                continue;
            };
            let idx = [i, j, k, l];
            let pitch = (b[l].cx - b[i].cx) / 3.0;
            let dev: f32 = (0..3)
                .map(|n| ((b[idx[n + 1]].cx - b[idx[n]].cx) - pitch).abs() / pitch)
                .fold(0.0, f32::max);
            let ys: f32 = idx.iter().map(|&n| (b[n].cy - b[i].cy).abs()).sum::<f32>() / ytol;
            cands.push((dev + 0.1 * ys, idx));
        }
    }
    cands.sort_by(|p, q| p.0.total_cmp(&q.0));
    let mut used = vec![false; b.len()];
    let mut groups = Vec::new();
    for (_, idx) in cands {
        if idx.iter().any(|&n| used[n]) {
            continue;
        }
        idx.iter().for_each(|&n| used[n] = true);
        let bl = idx.map(|n| b[n]);
        groups.push(Group {
            cx: bl.iter().map(|x| x.cx).sum::<f32>() / 4.0,
            cy: bl.iter().map(|x| x.cy).sum::<f32>() / 4.0,
            diam: bl.iter().map(|x| x.diam).sum::<f32>() / 4.0,
            pitch: (bl[3].cx - bl[0].cx) / 3.0,
            blobs: bl,
        });
    }
    groups
}

/// Our group (spec 03 side detection, step 2): decided when the largest group is also the one
/// nearest the horizontal centre. `None` means undecided (press correlation or ask the user).
pub fn pick_ours(groups: &[Group], stream_w: u32, side: Side) -> Option<usize> {
    let largest = (0..groups.len()).max_by(|&a, &b| groups[a].diam.total_cmp(&groups[b].diam))?;
    let pick = |key: &dyn Fn(&Group) -> f32| {
        (0..groups.len()).min_by(|&a, &b| key(&groups[a]).total_cmp(&key(&groups[b])))
    };
    let mid = stream_w as f32 / 2.0;
    match side {
        Side::Centre => {
            let centred = pick(&|g| (g.cx - mid).abs())?;
            (centred == largest).then_some(largest)
        }
        Side::Left => pick(&|g| g.cx),
        Side::Right => pick(&|g| -g.cx),
    }
}

/// Until notes confirm it: notes come from the side of the receptors with more room.
pub fn guess_scroll(g: &Group, stream_h: u32) -> Scroll {
    if g.cy < stream_h as f32 / 2.0 {
        Scroll::Up
    } else {
        Scroll::Down
    }
}

/// Full-resolution median luma of `r`, row-major `r.w` wide.
fn median_box(frames: &[FrameView], r: Rect) -> Vec<u8> {
    median_luma(frames, r, 1).0
}

/// Refine centres at full resolution (spec 03 step 5). x is the centroid of the receptor's
/// pixels; y the fill-weighted row centroid over the strip columns, as for heads.
pub fn layout_for(
    g: &Group,
    frames: &[FrameView],
    scroll: Scroll,
    stream: [u32; 2],
    cfg: &DetectConfig,
) -> Layout {
    let region = frames[0].region;
    let sw = (cfg.strip_width_frac * g.pitch).max(6.0).round() as i32;
    let mut receptors = [[0.0f32; 2]; 4];
    let mut run_len = [g.diam; 4];
    let mut run_shape = [[-g.diam / 2.0, 0.0, g.diam / 2.0]; 4];
    for (lane, b) in g.blobs.iter().enumerate() {
        let want = Rect::new(
            (b.cx - g.pitch / 2.0) as i32,
            (b.cy - 0.75 * g.diam) as i32,
            g.pitch as u32,
            (1.5 * g.diam) as u32,
        );
        let Some(r) = want.intersect(&region) else {
            receptors[lane] = [b.cx, b.cy];
            continue;
        };
        let l = median_box(frames, r);
        let mut sorted = l.clone();
        sorted.sort_unstable();
        let lo = sorted[sorted.len() / 10] as i32;
        let hi = sorted[sorted.len() * 19 / 20] as i32;
        let t = (lo + hi) / 2;
        let on = |v: u8| {
            if b.bright {
                v as i32 > t
            } else {
                (v as i32) < t
            }
        };
        let w = r.w as usize;
        // x: middle of the receptor's column extent (arrow centroids sit off-centre).
        let cols: Vec<usize> = (0..w)
            .filter(|&x| (0..r.h as usize).any(|y| on(l[y * w + x])))
            .collect();
        let cx = match (cols.first(), cols.last()) {
            (Some(&a), Some(&z)) => r.x as f32 + (a + z) as f32 / 2.0 + 0.5,
            _ => b.cx,
        };
        // Row fill over the strip columns; take the longest run of filled rows.
        let x0 = ((cx - sw as f32 / 2.0).round() as i32 - r.x).clamp(0, r.w as i32) as usize;
        let x1 = (x0 + sw as usize).min(w);
        let need = (cfg.head_fill * sw as f32).ceil() as u32;
        let fills: Vec<u16> = (0..r.h as usize)
            .map(|y| l[y * w + x0..y * w + x1].iter().filter(|&&v| on(v)).count() as u16)
            .collect();
        let mut runs = Vec::new();
        super::runs(&fills, need as u16, r.y, &mut runs);
        let rows: Vec<usize> = (0..r.h as usize)
            .filter(|&y| l[y * w..(y + 1) * w].iter().any(|&v| on(v)))
            .collect();
        let cy = match (rows.first(), rows.last()) {
            (Some(&a), Some(&z)) => r.y as f32 + (a + z) as f32 / 2.0,
            _ => b.cy - 0.5,
        };
        receptors[lane] = [cx, cy];
        if let Some(q) = runs.iter().max_by_key(|q| q.b - q.a) {
            run_len[lane] = (q.b - q.a + 1) as f32;
            run_shape[lane] = [q.a as f32 - cy, q.wy / q.w - cy, q.b as f32 - cy];
        }
    }
    Layout {
        receptors,
        size: g.diam,
        run_len,
        run_shape,
        scroll,
        stream,
    }
}

fn median_i16(v: &mut [i16]) -> i16 {
    let m = v.len() / 2;
    *v.select_nth_unstable(m).1
}

fn median_opp(px: &[Opp]) -> Opp {
    let mut c: Vec<i16> = px.iter().map(|p| p.0).collect();
    let a = median_i16(&mut c);
    c = px.iter().map(|p| p.1).collect();
    let b = median_i16(&mut c);
    c = px.iter().map(|p| p.2).collect();
    Opp(a, b, median_i16(&mut c))
}

fn p95_dist(px: &[Opp], c: Opp) -> i32 {
    let mut d: Vec<i32> = px.iter().map(|p| p.dist(c)).collect();
    if d.is_empty() {
        return 0;
    }
    let k = (d.len() * 95 / 100).min(d.len() - 1);
    *d.select_nth_unstable(k).1
}

struct HeadRun {
    colour: Opp,
    len: u32,
    px: Vec<Opp>,
}

/// Colour learning (spec 03). Feed frames that show notes moving; `finish` needs 6 head runs a lane.
pub struct ColourLearner {
    layout: Layout,
    cfg: DetectConfig,
    strips: [Rect; 4],
    /// Median background luma per strip pixel.
    pub bg: [Vec<u8>; 4],
    heads: [Vec<HeadRun>; 4],
    trail_px: Vec<Opp>,
    trail_runs: usize,
}

impl ColourLearner {
    /// `bg_frames`: ~20 frames spread over the whole learning window (10-20 s), for the
    /// background median. A shorter spread lets one long hold's trail become background.
    pub fn new(layout: Layout, cfg: DetectConfig, bg_frames: &[FrameView]) -> Self {
        let strips = [0, 1, 2, 3].map(|l| layout.strip(l, &cfg));
        let bg = strips.map(|s| match s.intersect(&bg_frames[0].region) {
            Some(r) if r == s => median_box(bg_frames, s),
            _ => vec![0; (s.w * s.h) as usize],
        });
        Self {
            layout,
            cfg,
            strips,
            bg,
            heads: Default::default(),
            trail_px: Vec::new(),
            trail_runs: 0,
        }
    }

    /// Head runs collected per lane so far.
    pub fn counts(&self) -> [usize; 4] {
        [0, 1, 2, 3].map(|l| self.heads[l].len())
    }

    /// Long (head + trail) runs seen so far; the trail colour needs 3.
    pub fn trail_runs(&self) -> usize {
        self.trail_runs
    }

    pub fn feed(&mut self, f: &FrameView) {
        for lane in 0..4 {
            let s = self.strips[lane];
            if s.intersect(&f.region) != Some(s) {
                continue;
            }
            let zone = self.layout.receptor_zone(lane);
            let w = s.w as usize;
            let mut fills = vec![0u16; s.h as usize];
            let mut rows: Vec<Vec<Opp>> = vec![Vec::new(); s.h as usize];
            for (ri, row) in rows.iter_mut().enumerate() {
                let y = s.y + ri as i32;
                for xi in 0..w {
                    let (r, g, b) = f.px(s.x + xi as i32, y);
                    let o = Opp::from_rgb(r, g, b);
                    let bgl = self.bg[lane][ri * w + xi] as i32;
                    if o.chroma() > self.cfg.learn_chroma
                        || (o.2 as i32 - bgl).abs() > self.cfg.bg_tol
                    {
                        row.push(o);
                    }
                }
                fills[ri] = row.len() as u16;
            }
            let mut runs = Vec::new();
            super::runs(
                &fills,
                (self.cfg.head_fill * w as f32).ceil() as u16,
                s.y,
                &mut runs,
            );
            for q in runs {
                let len = (q.b - q.a + 1) as f32;
                // Head candidates must not be cut by the strip ends or touch the receptor zone;
                // trails usually are, and that is fine.
                let whole =
                    q.a > s.y && q.b < s.y + s.h as i32 - 1 && !(q.a < zone.1 && q.b >= zone.0);
                let px: Vec<Opp> = (q.a..=q.b)
                    .flat_map(|y| rows[(y - s.y) as usize].iter().copied())
                    .collect();
                if whole && (0.3 * self.layout.size..=1.6 * self.layout.size).contains(&len) {
                    self.heads[lane].push(HeadRun {
                        colour: median_opp(&px),
                        len: len as u32,
                        px: px.into_iter().step_by(3).collect(),
                    });
                } else if len >= 1.8 * self.layout.size {
                    self.trail_runs += 1;
                    self.trail_px.extend(px.into_iter().step_by(5));
                }
            }
        }
    }

    pub fn finish(&self) -> Result<Colours, String> {
        let short: Vec<usize> = (0..4).filter(|&l| self.heads[l].len() < 6).collect();
        if !short.is_empty() {
            return Err(format!(
                "too few notes seen in lanes {short:?} (need 6 each)"
            ));
        }
        let cfg = &self.cfg;
        let mut heads = [Opp::default(); 4];
        let mut head_tol = [0; 4];
        let mut head_len = [0.0; 4];
        for lane in 0..4 {
            let runs = &self.heads[lane];
            let colours: Vec<Opp> = runs.iter().map(|r| r.colour).collect();
            let c = median_opp(&colours);
            let inliers: Vec<&HeadRun> = runs
                .iter()
                .filter(|r| r.colour.dist(c) <= cfg.head_tol_max)
                .collect();
            let px: Vec<Opp> = inliers.iter().flat_map(|r| r.px.iter().copied()).collect();
            heads[lane] = c;
            head_tol[lane] = p95_dist(&px, c).clamp(cfg.head_tol_min, cfg.head_tol_max);
            let mut lens: Vec<u32> = inliers.iter().map(|r| r.len).collect();
            lens.sort_unstable();
            head_len[lane] = lens
                .get(lens.len() / 2)
                .map_or(self.layout.run_len[lane], |&l| l as f32);
        }
        let tp: Vec<Opp> = self
            .trail_px
            .iter()
            .copied()
            .filter(|p| (0..4).all(|l| p.dist(heads[l]) > head_tol[l]))
            .collect();
        // A trail colour the background could pass for is useless (and would freeze the
        // detector's background model), so it must stand clear of the median background.
        let mut bgl: Vec<u8> = self.bg.iter().flatten().copied().collect();
        let bg_y = if bgl.is_empty() {
            0
        } else {
            median_u8(&mut bgl) as i32
        };
        let t = (self.trail_runs >= 3 && tp.len() >= 200).then(|| median_opp(&tp));
        let (trail, trail_tol) = match t {
            Some(t)
                if (t.2 as i32 - bg_y).abs() > 2 * cfg.bg_tol || t.chroma() > cfg.learn_chroma =>
            {
                (
                    Some(t),
                    p95_dist(&tp, t).clamp(cfg.trail_tol_min, cfg.trail_tol_max),
                )
            }
            _ => (None, cfg.trail_tol_min),
        };
        Ok(Colours {
            heads,
            head_tol,
            trail,
            trail_tol,
            head_len,
        })
    }
}

/// Receptor presence check (spec 03 session state): NCC of each receptor box against a template.
pub struct Presence {
    boxes: [Rect; 4],
    templates: [Vec<f32>; 4],
    min: f32,
}

fn zero_mean(v: impl Iterator<Item = f32>) -> Vec<f32> {
    let v: Vec<f32> = v.collect();
    let m = v.iter().sum::<f32>() / v.len().max(1) as f32;
    let mut v: Vec<f32> = v.into_iter().map(|x| x - m).collect();
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-3);
    v.iter_mut().for_each(|x| *x /= n);
    v
}

impl Presence {
    /// Templates are 2r x 2r boxes from the median of `frames`.
    pub fn new(layout: &Layout, frames: &[FrameView], cfg: &DetectConfig) -> Self {
        let r = layout.radius();
        let boxes = layout.receptors.map(|[x, y]| {
            Rect::new(
                (x - r) as i32,
                (y - r) as i32,
                (2.0 * r) as u32,
                (2.0 * r) as u32,
            )
        });
        let templates = boxes.map(|b| match b.intersect(&frames[0].region) {
            Some(i) if i == b => zero_mean(median_box(frames, b).into_iter().map(|v| v as f32)),
            _ => Vec::new(),
        });
        Self {
            boxes,
            templates,
            min: cfg.receptor_ncc_min,
        }
    }

    /// Present if NCC >= min on 3 of 4 lanes.
    pub fn check(&self, f: &FrameView) -> bool {
        let mut ok = 0;
        for (b, t) in self.boxes.iter().zip(&self.templates) {
            if t.is_empty() || b.intersect(&f.region) != Some(*b) {
                continue;
            }
            let v = zero_mean((0..b.h as i32).flat_map(|y| {
                (0..b.w as i32).map(move |x| {
                    let (r, g, bb) = f.px(b.x + x, b.y + y);
                    (r as f32 + 2.0 * g as f32 + bb as f32) / 4.0
                })
            }));
            let ncc: f32 = v.iter().zip(t).map(|(a, b)| a * b).sum();
            if ncc >= self.min {
                ok += 1;
            }
        }
        ok >= 3
    }
}
