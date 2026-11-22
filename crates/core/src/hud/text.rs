#![allow(clippy::needless_range_loop)]
//! Text mask, line finding, stack finding and column-projection segmentation.
//! Colour-agnostic: text is "bright" (max channel) with a dark outline; the threshold
//! is a fraction of the region's own 98th percentile brightness.

use crate::frame::{FrameView, Rect};

pub const CW: usize = 12;
pub const CH: usize = 16;
pub const WW: usize = 48;
/// A region whose brightest 2% is below this holds no text.
const MIN_HI: u8 = 90;

/// Perceived brightness. Not max-channel: saturated pure red (stage glow) must stay below
/// the coral "Miss" text.
#[inline(always)]
fn luma(r: u8, g: u8, b: u8) -> u8 {
    ((r as u32 * 77 + g as u32 * 150 + b as u32 * 29) >> 8) as u8
}

#[derive(Clone, Debug)]
pub struct Cell {
    pub x0: i32,
    pub x1: i32,
    pub y0: i32,
    pub y1: i32,
    /// width / reference height
    pub ar: f32,
    pub bits: [u8; CW * CH],
}

#[derive(Clone, Debug)]
pub struct Row {
    pub cells: Vec<Cell>,
    pub ref_h: f32,
}

impl Row {
    /// Cells of a label of `words` words: everything before the `words`-th wide gap
    /// (the value starts after it). `None` if the row has fewer gaps.
    pub fn label_len(&self, words: usize) -> Option<usize> {
        let mut seen = 0;
        for i in 1..self.cells.len() {
            if (self.cells[i].x0 - self.cells[i - 1].x1) as f32 >= 0.2 * self.ref_h {
                seen += 1;
                if seen == words {
                    return Some(i);
                }
            }
        }
        None
    }
}

/// Reusable buffers; one per reader thread.
#[derive(Default)]
pub struct Segmenter {
    v: Vec<u8>,
    mask: Vec<bool>,
    w: usize,
    h: usize,
    ox: i32,
    oy: i32,
    ref_h: f32,
    ref_top: usize,
    ref_bot: usize,
}

fn clip(f: &FrameView, r: Rect) -> Option<Rect> {
    f.region.intersect(&r)
}

impl Segmenter {
    fn load(&mut self, f: &FrameView, r: Rect) -> bool {
        let Some(r) = clip(f, r) else { return false };
        self.w = r.w as usize;
        self.h = r.h as usize;
        self.ox = r.x;
        self.oy = r.y;
        self.v.clear();
        for y in 0..self.h {
            let i = (r.y - f.region.y) as usize * f.stride
                + (y * f.stride)
                + (r.x - f.region.x) as usize * 4;
            for x in 0..self.w {
                let p = &f.data[i + x * 4..i + x * 4 + 3];
                self.v.push(luma(p[2], p[1], p[0]));
            }
        }
        true
    }

    fn hi(&self) -> u8 {
        let mut hist = [0u32; 256];
        for &v in &self.v {
            hist[v as usize] += 1;
        }
        let want = (self.v.len() as u32 * 2) / 100;
        let mut acc = 0;
        for i in (0..256).rev() {
            acc += hist[i];
            if acc > want {
                return i as u8;
            }
        }
        0
    }

    fn make_mask(&mut self, frac: f32) -> bool {
        let hi = self.hi();
        if hi < MIN_HI {
            return false;
        }
        // Pass 1: loose threshold finds groups of columns. Pass 2: inside each group the
        // threshold is a fraction of that group's own peak, so a dim label and a bright
        // value on one line (different colours) both survive, and touching glyphs split.
        let (w, h) = (self.w, self.h);
        let t1 = (hi as f32 * 0.45) as u8;
        self.mask.clear();
        self.mask.resize(w * h, false);
        let col_ink = |s: &Self, x: usize| (0..h).any(|y| s.v[y * w + x] >= t1);
        let mut x = 0;
        while x < w {
            if !col_ink(self, x) {
                x += 1;
                continue;
            }
            let x0 = x;
            while x < w && col_ink(self, x) {
                x += 1;
            }
            // plateau of the text body (75th percentile of its ink), not the brightest speck
            let mut ink: Vec<u8> = (0..h)
                .flat_map(|y| self.v[y * w + x0..y * w + x].iter().copied())
                .filter(|&v| v >= t1)
                .collect();
            ink.sort_unstable();
            let peak = ink.get(ink.len() * 3 / 4).copied().unwrap_or(0);
            let t = (peak as f32 * frac) as u8;
            for y in 0..h {
                for xx in x0..x {
                    self.mask[y * w + xx] = self.v[y * w + xx] >= t;
                }
            }
        }
        true
    }

    /// Splits one line of text in `rect` into glyph cells by column projection.
    /// `None` when the region holds no text.
    pub fn segment(&mut self, f: &FrameView, rect: Rect) -> Option<Row> {
        if !self.load(f, rect) || !self.make_mask(0.78) {
            return None;
        }
        let (w, h) = (self.w, self.h);
        let row_ink = |y: usize| self.mask[y * w..(y + 1) * w].iter().any(|&b| b);
        let ytop = (0..h).find(|&y| row_ink(y))?;
        let ybot = (0..h).rev().find(|&y| row_ink(y))?;
        let mut runs: Vec<(usize, usize)> = vec![];
        let mut start = None;
        for x in 0..=w {
            let ink = x < w && (ytop..=ybot).any(|y| self.mask[y * w + x]);
            match (ink, start) {
                (true, None) => start = Some(x),
                (false, Some(s)) => {
                    runs.push((s, x));
                    start = None;
                }
                _ => {}
            }
        }
        // tight vertical extent per run
        let mut cells: Vec<(usize, usize, usize, usize)> = vec![];
        for (x0, x1) in runs {
            let mut ink = 0;
            let (mut y0, mut y1) = (usize::MAX, 0);
            for y in ytop..=ybot {
                let n = self.mask[y * w + x0..y * w + x1]
                    .iter()
                    .filter(|&&b| b)
                    .count();
                if n > 0 {
                    y0 = y0.min(y);
                    y1 = y;
                    ink += n;
                }
            }
            if ink >= 3 {
                cells.push((x0, x1, y0, y1 + 1));
            }
        }
        if cells.is_empty() {
            return None;
        }
        let maxh = cells.iter().map(|c| c.3 - c.2).max().unwrap();
        let mut tops: Vec<usize> = vec![];
        let mut bots: Vec<usize> = vec![];
        for c in &cells {
            if (c.3 - c.2) * 10 >= maxh * 6 {
                tops.push(c.2);
                bots.push(c.3);
            }
        }
        tops.sort_unstable();
        bots.sort_unstable();
        let (rt, rb) = (
            tops[tops.len() / 2],
            bots[bots.len() / 2].max(tops[tops.len() / 2] + 1),
        );
        self.ref_h = (rb - rt) as f32;
        self.ref_top = rt;
        self.ref_bot = rb;
        let ox = self.ox;
        // Cells much wider than a glyph hold touching italic digits: cut them at the emptiest
        // column near each equal division. (Narrow merges, e.g. a dot against a digit, are
        // left to the atlas-guided split in `hud::classify_all`.)
        let glyph_w = 0.8 * self.ref_h;
        let mut out: Vec<Cell> = Vec::with_capacity(cells.len());
        for (x0, x1, _, _) in cells {
            let w = (x1 - x0) as f32;
            let n = if w > 1.25 * self.ref_h {
                (w / glyph_w).round().max(2.0) as usize
            } else {
                1
            };
            let mut prev = x0;
            for i in 1..=n {
                let end = if i == n {
                    x1
                } else {
                    let want = x0 + (x1 - x0) * i / n;
                    let win = (w * 0.15) as usize + 1;
                    let col = |x: usize| (rt..rb).filter(|&y| self.mask[y * self.w + x]).count();
                    (want.saturating_sub(win).max(prev + 1)..(want + win).min(x1 - 1))
                        .min_by_key(|&x| (col(x), x.abs_diff(want)))
                        .unwrap_or(want)
                };
                out.extend(self.cell_between(ox + prev as i32, ox + end as i32));
                prev = end;
            }
        }
        Some(Row {
            cells: out,
            ref_h: self.ref_h,
        })
    }

    /// Cell over absolute columns `x0..x1` of the last segmented row (used to split a cell
    /// that holds touching glyphs). `None` if the span holds no ink.
    pub fn cell_between(&self, x0: i32, x1: i32) -> Option<Cell> {
        let (a, b) = (
            (x0 - self.ox).max(0) as usize,
            ((x1 - self.ox) as usize).min(self.w),
        );
        if b <= a {
            return None;
        }
        let (mut y0, mut y1) = (usize::MAX, 0);
        for y in 0..self.h {
            if self.mask[y * self.w + a..y * self.w + b].iter().any(|&m| m) {
                y0 = y0.min(y);
                y1 = y + 1;
            }
        }
        if y1 == 0 {
            return None;
        }
        let mut bits = [0u8; CW * CH];
        self.resample(a, b, self.ref_top, self.ref_bot, CW, CH, &mut bits);
        Some(Cell {
            x0: self.ox + a as i32,
            x1: self.ox + b as i32,
            y0: self.oy + y0 as i32,
            y1: self.oy + y1 as i32,
            ar: (b - a) as f32 / self.ref_h,
            bits,
        })
    }

    /// Ink in column `x` (absolute) within the reference row range.
    pub fn col_ink(&self, x: i32) -> usize {
        let xr = (x - self.ox) as usize;
        if xr >= self.w {
            return 0;
        }
        (self.ref_top..self.ref_bot)
            .filter(|&y| self.mask[y * self.w + xr])
            .count()
    }

    /// Whole-word bitmap over cells `0..k` of the last segmented row, plus its aspect.
    pub fn word_bits(&self, row: &Row, k: usize) -> Option<([u8; WW * CH], f32)> {
        let cs = row.cells.get(..k).filter(|c| !c.is_empty())?;
        let x0 = (cs[0].x0 - self.ox) as usize;
        let x1 = (cs[k - 1].x1 - self.ox) as usize;
        let y0 = (cs.iter().map(|c| c.y0).min()? - self.oy) as usize;
        let y1 = (cs.iter().map(|c| c.y1).max()? - self.oy) as usize;
        let mut bits = [0u8; WW * CH];
        self.resample(x0, x1, y0, y1, WW, CH, &mut bits);
        Some((bits, (x1 - x0) as f32 / (y1 - y0).max(1) as f32))
    }

    #[allow(clippy::too_many_arguments)]
    fn resample(
        &self,
        x0: usize,
        x1: usize,
        y0: usize,
        y1: usize,
        ow: usize,
        oh: usize,
        out: &mut [u8],
    ) {
        let (bw, bh) = (x1 - x0, y1 - y0);
        for oy in 0..oh {
            let sy0 = y0 + oy * bh / oh;
            let sy1 = (y0 + (oy + 1) * bh / oh).max(sy0 + 1).min(y1.max(sy0 + 1));
            for ox in 0..ow {
                let sx0 = x0 + ox * bw / ow;
                let sx1 = (x0 + (ox + 1) * bw / ow).max(sx0 + 1).min(x1.max(sx0 + 1));
                let (mut on, mut n) = (0u32, 0u32);
                for y in sy0..sy1 {
                    for x in sx0..sx1 {
                        if x < self.w && y < self.h {
                            on += self.mask[y * self.w + x] as u32;
                        }
                        n += 1;
                    }
                }
                out[oy * ow + ox] = (on * 255 / n.max(1)) as u8;
            }
        }
    }

    /// Splits a block of stacked text lines into row rects by row projection.
    /// Columns with an unbroken ink run longer than one row pitch (`rect.h / n_rows`) are
    /// ignored: a long hold-note trail can cross the panel.
    pub fn rows_in(&mut self, f: &FrameView, rect: Rect, n_rows: usize) -> Vec<Rect> {
        if !self.load(f, rect) {
            return vec![];
        }
        let hi = self.hi();
        if hi < MIN_HI {
            return vec![];
        }
        let t = (hi as f32 * 0.45).max(60.0) as u8;
        let (w, h) = (self.w, self.h);
        let pitch = h / n_rows.max(1);
        let long_col: Vec<bool> = (0..w)
            .map(|x| {
                let (mut run, mut best) = (0, 0);
                for y in 0..h {
                    run = if self.v[y * w + x] >= 55 { run + 1 } else { 0 };
                    best = best.max(run);
                }
                n_rows > 1 && best > pitch
            })
            .collect();
        let skip = |x: usize| {
            long_col[x.saturating_sub(3)..(x + 4).min(w)]
                .iter()
                .any(|&b| b)
        };
        let ink: Vec<bool> = (0..h)
            .map(|y| (0..w).any(|x| !skip(x) && self.v[y * w + x] >= t))
            .collect();
        let mut runs: Vec<(usize, usize)> = vec![];
        let mut s = None;
        for y in 0..=h {
            match (y < h && ink[y], s) {
                (true, None) => s = Some(y),
                (false, Some(a)) => {
                    runs.push((a, y));
                    s = None;
                }
                _ => {}
            }
        }
        let maxh = runs.iter().map(|r| r.1 - r.0).max().unwrap_or(0);
        let mut merged: Vec<(usize, usize)> = vec![];
        for r in runs {
            match merged.last_mut() {
                Some(l) if (r.0 - l.1) * 10 < maxh * 3 => l.1 = r.1,
                _ => merged.push(r),
            }
        }
        // Something else (a hold trail, a glow) bridged rows: the stack is evenly spaced and
        // `rect` is first row top to last row bottom, so lay the rows out from the median height.
        if merged.len() != n_rows && n_rows > 1 && !merged.is_empty() {
            let mut hs: Vec<usize> = merged.iter().map(|r| r.1 - r.0).collect();
            hs.sort_unstable();
            let rh = hs[(hs.len() - 1) / 2];
            let top = merged[0].0;
            let last = merged[merged.len() - 1].1;
            let span = last.saturating_sub(top).max(rh);
            let pitch = (span.saturating_sub(rh)) as f32 / (n_rows - 1) as f32;
            merged = (0..n_rows)
                .map(|i| {
                    (
                        top + (i as f32 * pitch) as usize,
                        top + (i as f32 * pitch) as usize + rh,
                    )
                })
                .collect();
        }
        merged
            .into_iter()
            .map(|(a, b)| Rect::new(self.ox, self.oy + a as i32, w as u32, (b - a) as u32))
            .collect()
    }
}

/// Text-line candidates over the whole frame: connected components of bright pixels,
/// grouped left to right. Rects are tight around the big components; a leading "-" or a
/// trailing "." are smaller than the size filter, so `pad_line` before segmenting a single line.
pub fn find_lines(f: &FrameView, thr: u8) -> Vec<Rect> {
    let (w, h) = (f.region.w as usize, f.region.h as usize);
    let mut lab = vec![0u32; w * h];
    let mut parent: Vec<u32> = vec![0];
    fn root(p: &mut [u32], mut a: u32) -> u32 {
        while p[a as usize] != a {
            p[a as usize] = p[p[a as usize] as usize];
            a = p[a as usize];
        }
        a
    }
    for y in 0..h {
        for x in 0..w {
            let i = y * f.stride + x * 4;
            if luma(f.data[i + 2], f.data[i + 1], f.data[i]) < thr {
                continue;
            }
            let mut l = 0u32;
            for (dx, dy) in [(-1i32, 0i32), (-1, -1), (0, -1), (1, -1)] {
                let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                if nx < 0 || ny < 0 || nx >= w as i32 {
                    continue;
                }
                let n = lab[ny as usize * w + nx as usize];
                if n == 0 {
                    continue;
                }
                if l == 0 {
                    l = n;
                } else {
                    let (a, b) = (root(&mut parent, l), root(&mut parent, n));
                    if a != b {
                        parent[b as usize] = a;
                    }
                }
            }
            if l == 0 {
                l = parent.len() as u32;
                parent.push(l);
            }
            lab[y * w + x] = l;
        }
    }
    // bbox per root
    let mut bb: Vec<[i32; 4]> = vec![[i32::MAX, i32::MAX, -1, -1]; parent.len()];
    for y in 0..h {
        for x in 0..w {
            let l = lab[y * w + x];
            if l == 0 {
                continue;
            }
            let r = root(&mut parent, l) as usize;
            let b = &mut bb[r];
            b[0] = b[0].min(x as i32);
            b[1] = b[1].min(y as i32);
            b[2] = b[2].max(x as i32);
            b[3] = b[3].max(y as i32);
        }
    }
    let (hmin, hmax) = ((h as f32 * 0.006) as i32, (h as f32 * 0.09) as i32);
    let mut comps: Vec<[i32; 4]> = bb
        .into_iter()
        .filter(|b| {
            b[2] >= 0 && (hmin..=hmax).contains(&(b[3] - b[1] + 1)) && b[2] - b[0] < hmax * 3
        })
        .collect();
    comps.sort_by_key(|b| b[0]);
    let n = comps.len();
    let mut uf: Vec<u32> = (0..n as u32).collect();
    for i in 0..n {
        let (a, ha) = (comps[i], comps[i][3] - comps[i][1] + 1);
        for j in i + 1..n {
            let (b, hb) = (comps[j], comps[j][3] - comps[j][1] + 1);
            let gap = b[0] - a[2];
            if gap > hmax * 11 / 10 {
                // sorted by x0, but a's x1 may exceed later comps' x0 only within a glyph: stop
                if b[0] > a[2] + hmax * 2 {
                    break;
                }
                continue;
            }
            let ov = a[3].min(b[3]) - a[1].max(b[1]) + 1;
            let hm = ha.max(hb);
            if ov * 2 >= ha.min(hb) && gap <= hm && ha * 2 >= hb && hb * 2 >= ha {
                let (ra, rb) = (root(&mut uf, i as u32), root(&mut uf, j as u32));
                if ra != rb {
                    uf[rb as usize] = ra;
                }
            }
        }
    }
    let mut groups: Vec<([i32; 4], u32)> = vec![];
    let mut idx = std::collections::HashMap::new();
    for i in 0..n {
        let r = root(&mut uf, i as u32);
        let k = *idx.entry(r).or_insert_with(|| {
            groups.push(([i32::MAX, i32::MAX, -1, -1], 0));
            groups.len() - 1
        });
        let g = &mut groups[k];
        for (d, s) in g.0.iter_mut().zip(comps[i]).enumerate() {
            *s.0 = if d < 2 {
                (*s.0).min(s.1)
            } else {
                (*s.0).max(s.1)
            };
        }
        g.1 += 1;
    }
    let mut out: Vec<Rect> = groups
        .into_iter()
        .filter(|(_, c)| *c >= 1)
        .map(|(b, _)| {
            Rect::new(
                f.region.x + b[0],
                f.region.y + b[1],
                (b[2] - b[0] + 1) as u32,
                (b[3] - b[1] + 1) as u32,
            )
        })
        .collect();
    out.sort_by_key(|r| (r.y, r.x));
    out
}

/// Runs of >= `min_rows` left-aligned lines of similar height and regular pitch,
/// top to bottom. Returns indices into `lines`.
pub fn stacks(lines: &[Rect], min_rows: usize) -> Vec<Vec<usize>> {
    let mut used = vec![false; lines.len()];
    let mut out = vec![];
    for s in 0..lines.len() {
        if used[s] {
            continue;
        }
        let mut chain = vec![s];
        loop {
            let last = lines[*chain.last().unwrap()];
            let h = last.h as i32;
            let mut best: Option<usize> = None;
            for (k, l) in lines.iter().enumerate() {
                if used[k] || chain.contains(&k) {
                    continue;
                }
                let dy = l.y - last.y;
                let ok = dy * 10 >= h * 11
                    && dy <= h * 4
                    && (l.x - lines[s].x).abs() * 10 <= h * 8
                    && (l.h as i32 * 4 >= h * 3 && l.h as i32 * 3 <= h * 4)
                    && match chain.len() {
                        1 => true,
                        n => {
                            let pitch = (last.y - lines[chain[n - 2]].y) as f32;
                            (dy as f32 - pitch).abs() <= pitch * 0.2
                        }
                    };
                if ok && best.is_none_or(|b| lines[b].y > l.y) {
                    best = Some(k);
                }
            }
            match best {
                Some(b) => chain.push(b),
                None => break,
            }
        }
        if chain.len() >= min_rows {
            for &c in &chain {
                used[c] = true;
            }
            out.push(chain);
        }
    }
    out
}

pub fn union(rs: &[Rect]) -> Rect {
    let x0 = rs.iter().map(|r| r.x).min().unwrap_or(0);
    let y0 = rs.iter().map(|r| r.y).min().unwrap_or(0);
    let x1 = rs.iter().map(|r| r.x + r.w as i32).max().unwrap_or(0);
    let y1 = rs.iter().map(|r| r.y + r.h as i32).max().unwrap_or(0);
    Rect::new(x0, y0, (x1 - x0) as u32, (y1 - y0) as u32)
}

/// `lines` plus unions of up to 3 same-row neighbours with gaps under 2.5 line heights.
/// Wide-spaced fonts (the ms readout) come out of `find_lines` in pieces.
pub fn with_unions(lines: &[Rect]) -> Vec<Rect> {
    let mut out = lines.to_vec();
    let mut by_x: Vec<Rect> = lines.to_vec();
    by_x.sort_by_key(|r| r.x);
    for i in 0..by_x.len() {
        let mut cur = by_x[i];
        for b in by_x.iter().skip(i + 1).take(8) {
            let h = cur.h as i32;
            let same_row = ((b.y + b.h as i32 / 2) - (cur.y + h / 2)).abs() * 2 <= h;
            if same_row && b.x - (cur.x + cur.w as i32) <= h * 5 / 2 && b.x >= cur.x {
                cur = union(&[cur, *b]);
                out.push(cur);
                if out.len() > lines.len() * 4 {
                    return out;
                }
            }
        }
    }
    out
}

/// Room for the small glyphs (dash, dot) that `find_lines` leaves out, clipped to the frame.
pub fn pad_line(f: &FrameView, r: Rect) -> Rect {
    let (px, py) = (r.h as i32 * 3 / 4, r.h as i32 / 6 + 1);
    let p = Rect::new(
        r.x - px,
        r.y - py,
        (r.w as i32 + 2 * px) as u32,
        (r.h as i32 + 2 * py) as u32,
    );
    f.region.intersect(&p).unwrap_or(r)
}
