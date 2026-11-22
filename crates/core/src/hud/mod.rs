//! HUD reader (spec 03, "HUD reading"): glyph atlas, element finding and parsing.
//! No OCR: regions are found by structure (stacks of left-aligned text rows) and by
//! reading candidate lines with the atlas; the atlas is taught from known text.
//! Runs on the hud thread (10-20 Hz); nothing here spawns or does I/O except `Atlas::save/load`.

pub mod atlas;
pub mod readback;
mod text;

pub use atlas::{Atlas, Face};
pub use readback::{Matched, Readback, SentPress};

use crate::frame::{FrameView, Rect};
use text::{Segmenter, find_lines, pad_line, stacks, union, with_unions};

/// Bright-pixel threshold for finding text lines in a whole frame (game HUD).
const GAME_THR: u8 = 105;
/// Same for the results panel, whose grey "Bad" line is dimmer.
const RESULTS_THR: u8 = 90;
/// A row label may differ a bit more than a glyph (more pixels, more outline noise).
const LABEL_SLACK: f32 = 1.3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// Left panel: Accuracy, Highest Combo, Combo, Hits, Misses, Mean MS, KPS.
    Stats,
    /// Right of the playfield: Sick, Good, Ok, Bad, Miss.
    Counts,
    /// Results panel, left column: Sick .. Miss, Total Notes.
    ResCounts,
    /// Results panel, right column: Accuracy, Mean MS, Combo, Highest Combo.
    ResStats,
}

impl Layout {
    pub fn labels(self) -> &'static [&'static str] {
        match self {
            Layout::Stats => &[
                "Accuracy",
                "Highest Combo",
                "Combo",
                "Hits",
                "Misses",
                "Mean MS",
                "KPS",
            ],
            Layout::Counts => &["Sick", "Good", "Ok", "Bad", "Miss"],
            Layout::ResCounts => &["Sick", "Good", "Ok", "Bad", "Miss", "Total Notes"],
            Layout::ResStats => &["Accuracy", "Mean MS", "Combo", "Highest Combo"],
        }
    }
    pub fn face(self) -> Face {
        match self {
            Layout::Stats | Layout::Counts => Face::Stats,
            _ => Face::Results,
        }
    }
    fn thr(self) -> u8 {
        if self.face() == Face::Results {
            RESULTS_THR
        } else {
            GAME_THR
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Counts {
    pub sick: Option<u32>,
    pub good: Option<u32>,
    pub ok: Option<u32>,
    pub bad: Option<u32>,
    pub miss: Option<u32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StatsRead {
    pub accuracy: Option<f32>,
    pub highest_combo: Option<u32>,
    pub combo: Option<u32>,
    pub hits: Option<u32>,
    pub misses: Option<u32>,
    pub mean_ms: Option<f32>,
    pub kps: Option<u32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ResultsRead {
    pub counts: Counts,
    pub total_notes: Option<u32>,
    pub accuracy: Option<f32>,
    pub mean_ms: Option<f32>,
    pub combo: Option<u32>,
    pub highest_combo: Option<u32>,
}

/// One tick of the HUD reader. `None` = region unknown, blank or rejected.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HudRead {
    pub stats: Option<StatsRead>,
    pub counts: Option<Counts>,
    /// Signed ms exactly as the game shows it (late = negative, G4).
    pub ms: Option<f32>,
    /// The ms region held no text this tick (feed `Readback::on_blank`).
    pub ms_blank: bool,
    /// Scores by screen side: [left, right].
    pub scores: [Option<i32>; 2],
}

/// Where each element was found, in stream pixels. Stored in the profile's `[hud]`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Locations {
    pub stats: Option<Rect>,
    pub counts: Option<Rect>,
    pub ms: Option<Rect>,
    pub score_left: Option<Rect>,
    pub score_right: Option<Rect>,
    /// Results panel (left column); set by `locate_results`.
    pub results: Option<Rect>,
    pub results_stats: Option<Rect>,
}

fn arr(r: Option<Rect>) -> Option<[i32; 4]> {
    r.map(|r| [r.x, r.y, r.w as i32, r.h as i32])
}
fn rect(a: Option<[i32; 4]>) -> Option<Rect> {
    a.map(|a| Rect::new(a[0], a[1], a[2].max(0) as u32, a[3].max(0) as u32))
}

impl Locations {
    pub fn to_regions(&self) -> crate::config::HudRegions {
        crate::config::HudRegions {
            stats: arr(self.stats),
            counts: arr(self.counts),
            ms_readout: arr(self.ms),
            accuracy: None,
            score_left: arr(self.score_left),
            score_right: arr(self.score_right),
            ..Default::default()
        }
    }
    pub fn from_regions(r: &crate::config::HudRegions) -> Self {
        Self {
            stats: rect(r.stats),
            counts: rect(r.counts),
            ms: rect(r.ms_readout),
            score_left: rect(r.score_left),
            score_right: rect(r.score_right),
            ..Default::default()
        }
    }
}

// ---- parsers (pure) ----

/// `[-]digits.dd`
fn decimal(s: &str) -> Option<f32> {
    let b = s.strip_prefix('-').unwrap_or(s);
    let (i, d) = b.split_once('.')?;
    (!i.is_empty()
        && i.len() <= 5
        && d.len() == 2
        && i.bytes().chain(d.bytes()).all(|c| c.is_ascii_digit()))
    .then(|| s.parse().ok())?
}

pub fn parse_uint(s: &str) -> Option<u32> {
    (!s.is_empty() && s.len() <= 7 && s.bytes().all(|c| c.is_ascii_digit()))
        .then(|| s.parse().ok())?
}

pub fn parse_int(s: &str) -> Option<i32> {
    parse_uint(s.strip_prefix('-').unwrap_or(s))?;
    s.parse().ok()
}

/// "-143.26ms" (spaces already removed).
pub fn parse_ms(s: &str) -> Option<f32> {
    decimal(s.strip_suffix("ms")?)
}

/// "84.48%", 0..=100.
pub fn parse_pct(s: &str) -> Option<f32> {
    decimal(s.strip_suffix('%')?).filter(|v| (0.0..=100.0).contains(v))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Judgment {
    Sick,
    Good,
    Ok,
    Bad,
    Miss,
}

impl Judgment {
    pub fn parse(s: &str) -> Option<Self> {
        Some(
            match s
                .trim_end_matches(['!', ':', '.'])
                .to_ascii_lowercase()
                .as_str()
            {
                "sick" => Self::Sick,
                "good" => Self::Good,
                "ok" => Self::Ok,
                "bad" => Self::Bad,
                "miss" => Self::Miss,
                _ => return None,
            },
        )
    }
}

/// A row with one cell too many at an edge (icon next to a score): drop the edge cell whose
/// height differs most from the median.
fn drop_edge_outlier(mut row: text::Row, want: usize) -> text::Row {
    if row.cells.len() == want + 1 && want >= 2 {
        let mut hs: Vec<i32> = row.cells.iter().map(|c| c.y1 - c.y0).collect();
        hs.sort_unstable();
        let med = hs[hs.len() / 2];
        let dev = |c: &text::Cell| ((c.y1 - c.y0) - med).abs();
        let last = row.cells.len() - 1;
        row.cells
            .remove(if dev(&row.cells[0]) > dev(&row.cells[last]) {
                0
            } else {
                last
            });
    }
    row
}

#[derive(Clone, Copy, Debug)]
struct Glyph {
    ch: char,
    d: f32,
    x0: i32,
    x1: i32,
}

/// Per-cell best glyph. A cell that matches nothing may hold two touching glyphs (italic
/// digits, a "." against a digit): try every cut and keep the best pair that both match.
/// Cells still unmatched stay in the result with their (high) distance.
fn classify_all(
    atlas: &Atlas,
    seg: &Segmenter,
    face: Face,
    cells: &[text::Cell],
    max: f32,
) -> Option<Vec<Glyph>> {
    let mut out = Vec::with_capacity(cells.len());
    for c in cells {
        let g = atlas.classify(face, c)?;
        if g.1 <= max {
            out.push(Glyph {
                ch: g.0,
                d: g.1,
                x0: c.x0,
                x1: c.x1,
            });
            continue;
        }
        let w = c.x1 - c.x0;
        let mut best: Option<(Glyph, Glyph)> = None;
        for x in c.x0 + (w / 8).max(2)..c.x1 - (w / 8).max(2) {
            let (Some(a), Some(b)) = (seg.cell_between(c.x0, x), seg.cell_between(x, c.x1)) else {
                continue;
            };
            let (Some(ga), Some(gb)) = (atlas.classify(face, &a), atlas.classify(face, &b)) else {
                continue;
            };
            if ga.1 <= max * 0.85
                && gb.1 <= max * 0.85
                && best.is_none_or(|(p, q)| ga.1 + gb.1 < p.d + q.d)
            {
                best = Some((
                    Glyph {
                        ch: ga.0,
                        d: ga.1,
                        x0: a.x0,
                        x1: a.x1,
                    },
                    Glyph {
                        ch: gb.0,
                        d: gb.1,
                        x0: b.x0,
                        x1: b.x1,
                    },
                ));
            }
        }
        match best {
            Some((a, b)) => out.extend([a, b]),
            None => out.push(Glyph {
                ch: g.0,
                d: g.1,
                x0: c.x0,
                x1: c.x1,
            }),
        }
    }
    Some(out)
}

/// Text of `cells`, `None` if any glyph is unknown.
fn recognise(
    atlas: &Atlas,
    seg: &Segmenter,
    face: Face,
    cells: &[text::Cell],
    max: f32,
) -> Option<String> {
    let v = classify_all(atlas, seg, face, cells, max)?;
    v.iter()
        .all(|g| g.d <= max)
        .then(|| v.into_iter().map(|g| g.ch).collect())
}

/// Relative glyph widths, digit = 1, for assigning cells to known characters.
fn nominal(c: char) -> f32 {
    match c {
        '.' | ':' => 0.3,
        '-' => 0.6,
        '%' => 1.3,
        'm' => 1.4,
        's' => 0.9,
        _ => 1.0,
    }
}

/// How many characters each cell holds (1..=3), and how badly the cell widths fit the
/// characters' nominal widths (dynamic programming; lower is better).
fn fit(cells: &[text::Cell], chars: &[char]) -> Option<(Vec<usize>, f32)> {
    let (n, m) = (cells.len(), chars.len());
    if n == 0 || n > m {
        return None;
    }
    let pre: Vec<f32> = std::iter::once(0.0)
        .chain(chars.iter().scan(0.0, |a, &c| {
            *a += nominal(c);
            Some(*a)
        }))
        .collect();
    let s = cells.iter().map(|c| (c.x1 - c.x0) as f32).sum::<f32>() / pre[m];
    let mut dp = vec![vec![f32::INFINITY; m + 1]; n + 1];
    let mut back = vec![vec![0usize; m + 1]; n + 1];
    dp[0][0] = 0.0;
    for i in 0..n {
        let w = (cells[i].x1 - cells[i].x0) as f32 / s;
        for j in 0..m {
            if !dp[i][j].is_finite() {
                continue;
            }
            for c in 1..=3.min(m - j) {
                let cost = dp[i][j] + (w - (pre[j + c] - pre[j])).powi(2) + 0.05 * (c - 1) as f32;
                if cost < dp[i + 1][j + c] {
                    dp[i + 1][j + c] = cost;
                    back[i + 1][j + c] = c;
                }
            }
        }
    }
    if !dp[n][m].is_finite() {
        return None;
    }
    let mut counts = vec![0; n];
    let mut j = m;
    for i in (1..=n).rev() {
        counts[i - 1] = back[i][j];
        j -= back[i][j];
    }
    Some((counts, dp[n][m]))
}

/// Cells aligned one to one with `chars`, splitting cells that hold touching glyphs
/// (see `fit`); cuts go at the emptiest column near the proportional point.
fn explode(seg: &Segmenter, cells: &[text::Cell], chars: &[char]) -> Option<Vec<text::Cell>> {
    let (counts, _) = fit(cells, chars)?;
    let pre: Vec<f32> = std::iter::once(0.0)
        .chain(chars.iter().scan(0.0, |a, &c| {
            *a += nominal(c);
            Some(*a)
        }))
        .collect();
    let (mut out, mut j) = (vec![], 0);
    for (c, k) in cells.iter().zip(counts) {
        let (w, total) = ((c.x1 - c.x0) as f32, pre[j + k] - pre[j]);
        let mut prev = c.x0;
        for t in 1..=k {
            let end = if t == k {
                c.x1
            } else {
                let want = c.x0 + (w * (pre[j + t] - pre[j]) / total) as i32;
                let win = (w as i32 / 10).max(2);
                (want - win..=want + win)
                    .filter(|&x| x > prev && x < c.x1)
                    .min_by_key(|&x| (seg.col_ink(x), (x - want).abs()))
                    .unwrap_or(want)
            };
            out.push(seg.cell_between(prev, end)?);
            prev = end;
        }
        j += k;
    }
    Some(out)
}

// ---- reader ----

#[derive(Default)]
pub struct HudReader {
    pub atlas: Atlas,
    /// Config `[detect] glyph_max_sad`.
    pub max_sad: f32,
    pub locs: Locations,
    /// Our playfield's horizontal span (stream px, from the lane layout). Each player's ms
    /// readout sits near their own playfield, so only lines centred inside it are ours.
    pub playfield: Option<(i32, i32)>,
    seg: Segmenter,
}

impl HudReader {
    pub fn new(atlas: Atlas, glyph_max_sad: f32) -> Self {
        Self {
            atlas,
            max_sad: glyph_max_sad,
            ..Default::default()
        }
    }

    /// Our playfield span from the receptor centres and their spacing (`pitch`): one pitch
    /// beyond the outer lanes on each side.
    pub fn set_playfield(&mut self, receptor_x: [f32; 4], pitch: f32) {
        let (lo, hi) = receptor_x
            .iter()
            .fold((f32::MAX, f32::MIN), |(a, b), &x| (a.min(x), b.max(x)));
        self.playfield = Some(((lo - pitch) as i32, (hi + pitch) as i32));
    }

    fn in_playfield(&self, r: &Rect) -> bool {
        let c = r.x + r.w as i32 / 2;
        self.playfield.is_some_and(|(a, b)| (a..=b).contains(&c))
    }

    /// Whole line as text, spaces dropped. `None` if any glyph is unknown or no text.
    fn read_text(&mut self, f: &FrameView, r: Rect, face: Face) -> Option<String> {
        let row = self.seg.segment(f, r)?;
        recognise(&self.atlas, &self.seg, face, &row.cells, self.max_sad)
    }

    /// Like `read_text`, but an avatar or icon touching the line (scores sit next to one)
    /// may be dropped from either end. Returns the text and the tight rect around the glyphs
    /// that were kept, so later reads of that rect never see the decoration.
    fn read_text_trim(&mut self, f: &FrameView, r: Rect, face: Face) -> Option<(String, Rect)> {
        let row = self.seg.segment(f, r)?;
        let m = classify_all(&self.atlas, &self.seg, face, &row.cells, self.max_sad)
            .filter(|m| !m.is_empty())?;
        let mut ds: Vec<f32> = m.iter().map(|g| g.d).collect();
        ds.sort_by(f32::total_cmp);
        let limit = (3.0 * ds[(ds.len() - 1) / 2] + 0.05).max(0.2);
        let (mut a, mut b) = (0, m.len());
        if m.len() > 1 && m[0].d > limit {
            a = 1;
        }
        if m.len() > 1 && m[m.len() - 1].d > limit {
            b -= 1;
        }
        if m[a..b].iter().any(|g| g.d > self.max_sad) {
            return None;
        }
        let (x0, x1) = (m[a].x0, m[b - 1].x1);
        let pad = r.h as i32 / 8;
        Some((
            m[a..b].iter().map(|g| g.ch).collect(),
            Rect::new(x0 - pad, r.y, (x1 - x0 + 2 * pad) as u32, r.h),
        ))
    }

    /// Row with a known label: checks the label, returns the value text after it.
    fn read_row(&mut self, f: &FrameView, r: Rect, face: Face, label: &str) -> Option<String> {
        let row = self.seg.segment(f, r)?;
        let (_, bits, ar) = self.atlas.word(face, label)?;
        let k = row.label_len(label.split(' ').count())?;
        let (wb, war) = self.seg.word_bits(&row, k)?;
        if self.atlas.word_dist(&wb, war, &(*bits, ar)) > self.max_sad * LABEL_SLACK {
            return None;
        }
        recognise(&self.atlas, &self.seg, face, &row.cells[k..], self.max_sad)
    }

    fn read_layout(&mut self, f: &FrameView, r: Rect, l: Layout) -> Vec<Option<String>> {
        let rows = self.seg.rows_in(f, r, l.labels().len());
        if rows.len() != l.labels().len() {
            return vec![None; l.labels().len()];
        }
        rows.iter()
            .zip(l.labels())
            .map(|(&rr, lab)| self.read_row(f, rr, l.face(), lab))
            .collect()
    }

    pub fn read_stats(&mut self, f: &FrameView, r: Rect) -> StatsRead {
        let v = self.read_layout(f, r, Layout::Stats);
        let g = |i: usize| v[i].as_deref();
        StatsRead {
            accuracy: g(0).and_then(parse_pct),
            highest_combo: g(1).and_then(parse_uint),
            combo: g(2).and_then(parse_uint),
            hits: g(3).and_then(parse_uint),
            misses: g(4).and_then(parse_uint),
            mean_ms: g(5).and_then(parse_ms),
            kps: g(6).and_then(parse_uint),
        }
    }

    pub fn read_counts(&mut self, f: &FrameView, r: Rect) -> Counts {
        let v = self.read_layout(f, r, Layout::Counts);
        let g = |i: usize| v[i].as_deref().and_then(parse_uint);
        Counts {
            sick: g(0),
            good: g(1),
            ok: g(2),
            bad: g(3),
            miss: g(4),
        }
    }

    /// One tick on the known regions. Cheap; allocation only for the small strings.
    pub fn read(&mut self, f: &FrameView) -> HudRead {
        let mut out = HudRead::default();
        if let Some(r) = self.locs.stats {
            out.stats = Some(self.read_stats(f, r)).filter(|s| *s != StatsRead::default());
        }
        if let Some(r) = self.locs.counts {
            out.counts = Some(self.read_counts(f, r)).filter(|c| *c != Counts::default());
        }
        if let Some(r) = self.locs.ms {
            match self.seg.segment(f, r) {
                None => out.ms_blank = true,
                Some(_) => out.ms = self.read_text(f, r, Face::Ms).and_then(|t| parse_ms(&t)),
            }
        }
        for (i, r) in [self.locs.score_left, self.locs.score_right]
            .into_iter()
            .enumerate()
        {
            if let Some(r) = r {
                out.scores[i] = self
                    .read_text(f, r, Face::Score)
                    .and_then(|t| parse_int(&t));
            }
        }
        out
    }

    /// Results panel on a full frame: `None` unless both label stacks are found and match.
    /// Needs the Results face taught; one-off, not for the 10-20 Hz loop.
    pub fn read_results(&mut self, f: &FrameView) -> Option<ResultsRead> {
        let loc = self.locate_results(f);
        let (lr, rr) = (loc.results?, loc.results_stats);
        let v = self.read_layout(f, lr, Layout::ResCounts);
        let g = |i: usize| v[i].as_deref().and_then(parse_uint);
        let mut out = ResultsRead {
            counts: Counts {
                sick: g(0),
                good: g(1),
                ok: g(2),
                bad: g(3),
                miss: g(4),
            },
            total_notes: g(5),
            ..Default::default()
        };
        if let Some(rr) = rr {
            let v = self.read_layout(f, rr, Layout::ResStats);
            out.accuracy = v[0].as_deref().and_then(parse_pct);
            out.mean_ms = v[1].as_deref().and_then(parse_ms);
            out.combo = v[2].as_deref().and_then(parse_uint);
            out.highest_combo = v[3].as_deref().and_then(parse_uint);
        }
        // total must agree with the five counts when all are read
        let c = out.counts;
        if let (Some(a), Some(b), Some(c2), Some(d), Some(e), Some(t)) =
            (c.sick, c.good, c.ok, c.bad, c.miss, out.total_notes)
            && a + b + c2 + d + e != t
        {
            return None;
        }
        Some(out)
    }

    fn find_stack(&mut self, f: &FrameView, l: Layout) -> Option<(Rect, Vec<Rect>)> {
        let lines = find_lines(f, l.thr());
        let found: Vec<Vec<Rect>> = stacks(&lines, l.labels().len())
            .into_iter()
            // one extra row below is fine ("Final Score" under the results stats)
            .filter(|s| (l.labels().len()..=l.labels().len() + 1).contains(&s.len()))
            .map(|s| {
                s.into_iter()
                    .take(l.labels().len())
                    .map(|i| lines[i])
                    .collect()
            })
            .collect();
        let face = l.face();
        let taught = self.atlas.word(face, l.labels()[0]).is_some();
        for rows in found {
            if taught {
                let ok = rows
                    .iter()
                    .zip(l.labels())
                    .filter(|(r, lab)| self.read_row(f, **r, face, lab).is_some())
                    .count();
                if ok * 2 < rows.len() {
                    continue;
                }
            }
            return Some((union(&rows), rows));
        }
        None
    }

    /// Finds every element on a full frame and remembers the regions.
    /// Stats and counts by stack structure (verified by label when taught); ms and
    /// scores by reading the remaining lines with their faces. The ms readout is only
    /// looked for inside our playfield (`set_playfield`); the opponent's is ignored.
    pub fn locate(&mut self, f: &FrameView) -> Locations {
        let mut loc = Locations::default();
        let mut used: Vec<Rect> = vec![];
        for (l, slot) in [(Layout::Stats, 0), (Layout::Counts, 1)] {
            if let Some((r, rows)) = self.find_stack(f, l) {
                used.extend(rows);
                if slot == 0 {
                    loc.stats = Some(r)
                } else {
                    loc.counts = Some(r)
                }
            }
        }
        let lines: Vec<Rect> = with_unions(
            &find_lines(f, GAME_THR)
                .into_iter()
                .filter(|l| !used.iter().any(|u| u.intersect(l).is_some()))
                .collect::<Vec<_>>(),
        );
        let (mut scores, mut ms): (Vec<Rect>, Option<Rect>) = (vec![], None);
        for l in lines {
            let l = pad_line(f, l);
            if ms.is_none()
                && self.in_playfield(&l)
                && let Some(t) = self.read_text(f, l, Face::Ms)
                && parse_ms(&t).is_some()
            {
                let pad = l.h as i32 / 2;
                ms = Some(Rect::new(l.x - pad, l.y, l.w + 2 * pad as u32, l.h));
                continue;
            }
            if let Some((t, tight)) = self.read_text_trim(f, l, Face::Score)
                && parse_int(&t).is_some()
                && !scores.iter().any(|s: &Rect| s.intersect(&tight).is_some())
            {
                scores.push(tight);
            }
        }
        loc.ms = ms;
        scores.sort_by_key(|r| std::cmp::Reverse(r.h));
        let tallest = scores.first().map_or(0, |r| r.h);
        scores.retain(|r| r.h * 2 >= tallest);
        scores.truncate(2);
        scores.sort_by_key(|r| r.x);
        let mid = f.region.x + f.region.w as i32 / 2;
        for s in scores {
            if s.x + (s.w as i32) / 2 < mid {
                loc.score_left = Some(s)
            } else {
                loc.score_right = Some(s)
            }
        }
        self.locs = loc;
        loc
    }

    pub fn locate_results(&mut self, f: &FrameView) -> Locations {
        Locations {
            results: self.find_stack(f, Layout::ResCounts).map(|s| s.0),
            results_stats: self.find_stack(f, Layout::ResStats).map(|s| s.0),
            ..Default::default()
        }
    }

    // ---- teaching ----

    /// Teach a stack from its known row values, top to bottom (`"0.00%"`, `"15"`, `"0.00 ms"`...).
    /// Labels are learned too (cells before the value). Song start: every counter is 0, so
    /// `bootstrap_song_start`; later the runner calls this with `previous + 1` as a counter
    /// ticks to learn the remaining digits; Setup can call it with values the user types.
    pub fn teach_stack(&mut self, f: &FrameView, l: Layout, values: &[&str]) -> Result<(), String> {
        if values.len() != l.labels().len() {
            return Err(format!("{:?} needs {} values", l, l.labels().len()));
        }
        let (_, rows) = self.find_stack(f, l).ok_or("stack not found")?;
        let face = l.face();
        for ((r, lab), v) in rows.iter().zip(l.labels()).zip(values) {
            let chars: Vec<char> = v.chars().filter(|c| !c.is_whitespace()).collect();
            let row = self
                .seg
                .segment(f, *r)
                .ok_or_else(|| format!("{lab}: no text"))?;
            let k = row
                .label_len(lab.split(' ').count())
                .ok_or_else(|| format!("{lab}: label not found"))?;
            let (wb, war) = self.seg.word_bits(&row, k).ok_or("word bits")?;
            let cells = explode(&self.seg, &row.cells[k..], &chars).ok_or_else(|| {
                format!(
                    "{lab}: {} value cells cannot hold {} chars",
                    row.cells.len() - k,
                    chars.len()
                )
            })?;
            self.atlas.add_word(face, lab, k, wb, war);
            for (c, ch) in cells.iter().zip(chars) {
                self.atlas.add_glyph(face, ch, c);
            }
        }
        Ok(())
    }

    /// Song start (session `Armed`, before the first hit): all counters read zero.
    pub fn bootstrap_song_start(&mut self, f: &FrameView) -> Result<(), String> {
        self.teach_stack(
            f,
            Layout::Stats,
            &["0.00%", "0", "0", "0", "0", "0.00ms", "0"],
        )?;
        self.teach_stack(f, Layout::Counts, &["0"; 5])
    }

    /// Live teaching of the ms font, which differs from the stats panel's. On the frame where
    /// the stats panel's Hits first reads 1, its Mean MS is that hit's ms, so the readout in
    /// our playfield shows the same text (unverified, spec 12 G14). Teaches it, then keeps the
    /// new glyphs only if the readout then reads back as `mean_ms` there; else the atlas is
    /// left as it was. A readout that contradicts a glyph already known fails that read: the
    /// new glyph ties with the old one and the atlas rejects ties. Returns whether it was kept.
    pub fn teach_ms_first_hit(&mut self, f: &FrameView, mean_ms: f32) -> bool {
        let Some((a, b)) = self.playfield else {
            return false;
        };
        let text = format!("{mean_ms:.2}ms");
        let hint = Rect::new(a, f.region.y, (b - a).max(0) as u32, f.region.h);
        let (atlas, locs) = (self.atlas.clone(), self.locs);
        let ok = self.teach_lines(f, Face::Ms, &[&text], Some(hint)).is_ok()
            && self.locate(f).ms.is_some()
            && self.read(f).ms == Some(mean_ms);
        if !ok {
            self.atlas = atlas;
            self.locs = locs;
        }
        ok
    }

    /// Teach stand-alone lines (ms readout, scores) from their text. Lines are the frame's
    /// bright text lines whose cell count matches; with several texts of equal length the
    /// tallest lines are taken, left to right. `hint` limits the search to lines inside it
    /// (Setup: the user marks the number; the HUD has sprites that look like text), and only
    /// the part of a line inside the hint is read.
    pub fn teach_lines(
        &mut self,
        f: &FrameView,
        face: Face,
        texts: &[&str],
        hint: Option<Rect>,
    ) -> Result<(), String> {
        let stack_rows: Vec<Rect> = {
            let lines = find_lines(f, GAME_THR);
            stacks(&lines, 4)
                .into_iter()
                .flatten()
                .map(|i| lines[i])
                .collect()
        };
        let lines: Vec<Rect> = with_unions(
            &find_lines(f, GAME_THR)
                .into_iter()
                .filter(|l| !stack_rows.iter().any(|u| u.intersect(l).is_some()))
                .collect::<Vec<_>>(),
        )
        .into_iter()
        .map(|l| pad_line(f, l))
        .collect();
        // with a hint, only the part of each line inside it is used (cuts off an adjacent avatar)
        let lines: Vec<Rect> = lines
            .into_iter()
            .filter_map(|l| hint.map_or(Some(l), |h| h.intersect(&l)))
            .collect();
        let mut by_len: Vec<(usize, Vec<&str>)> = vec![];
        for t in texts {
            let n = t.chars().filter(|c| !c.is_whitespace()).count();
            match by_len.iter_mut().find(|b| b.0 == n) {
                Some(b) => b.1.push(t),
                None => by_len.push((n, vec![t])),
            }
        }
        for (n, ts) in by_len {
            let mut cands: Vec<(Rect, text::Row)> = vec![];
            for l in &lines {
                if let Some(row) = self.seg.segment(f, *l) {
                    let row = if face == Face::Score {
                        drop_edge_outlier(row, n)
                    } else {
                        row
                    };
                    // touching glyphs make fewer cells than characters
                    let mut hs: Vec<i32> = row.cells.iter().map(|c| c.y1 - c.y0).collect();
                    hs.sort_unstable();
                    let big_enough = hs
                        .get(hs.len() / 2)
                        .is_some_and(|&h| h as f32 >= 0.008 * f.region.h as f32);
                    if big_enough && (n.saturating_sub(2)..=n).contains(&row.cells.len()) {
                        cands.push((*l, row));
                    }
                }
            }
            if cands.len() < ts.len() {
                return Err(format!(
                    "{:?}: {} lines with {n} cells for {} texts",
                    face,
                    cands.len(),
                    ts.len()
                ));
            }
            let chars0: Vec<char> = ts[0].chars().filter(|c| !c.is_whitespace()).collect();
            let cost =
                |c: &(Rect, text::Row)| fit(&c.1.cells, &chars0).map_or(f32::INFINITY, |f| f.1);
            cands.sort_by(|a, b| cost(a).total_cmp(&cost(b)));
            let mut chosen: Vec<(Rect, text::Row)> = vec![];
            for c in cands {
                if chosen.len() < ts.len() && !chosen.iter().any(|d| d.0.intersect(&c.0).is_some())
                {
                    chosen.push(c);
                }
            }
            if chosen.len() < ts.len() {
                return Err(format!(
                    "{face:?}: only {} separate lines with {n} cells for {} texts",
                    chosen.len(),
                    ts.len()
                ));
            }
            let mut cands = chosen;
            cands.sort_by_key(|c| c.0.x);
            for ((r, _), t) in cands.iter().zip(ts) {
                let chars: Vec<char> = t.chars().filter(|c| !c.is_whitespace()).collect();
                let row = self.seg.segment(f, *r).ok_or("line vanished")?;
                let row = if face == Face::Score {
                    drop_edge_outlier(row, n)
                } else {
                    row
                };
                let cells = explode(&self.seg, &row.cells, &chars)
                    .ok_or_else(|| format!("{t}: cells do not fit"))?;
                for (c, ch) in cells.iter().zip(chars) {
                    self.atlas.add_glyph(face, ch, c);
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
