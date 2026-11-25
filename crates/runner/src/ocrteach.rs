//! OCR as a teacher for the HUD glyph reader (spec 03): OCR text lines in, trusted values out.
//! OCR misreads digits in the game's comic font (`5` as `S`, `1` as `)` or `I`), so a value is
//! only trusted when it parses after repair, at least two OCR passes agree on it, and the
//! panel's arithmetic holds (Hits = Sick + Good + Ok + Bad, Misses = Miss).

use arrowhead_core::frame::{FrameView, Rect};
use arrowhead_core::hud::{Counts, ResultsRead, StatsRead};

pub type Lines = [(String, Rect)];

/// Digit look-alikes in the game font, as OCR returns them. Only used inside value text.
fn repair(c: char) -> Option<char> {
    Some(match c {
        '0'..='9' | '.' | '-' | '%' => c,
        ',' => '.',
        'S' | 's' | '$' => '5',
        'O' | 'o' | 'D' | 'Q' => '0',
        'I' | 'l' | '|' | ')' | '(' | '!' | 'i' | 'J' => '1',
        'Z' | 'z' => '2',
        'G' | 'b' | 'C' => '6',
        'B' => '8',
        'Y' | 'y' => '4',
        'T' => '7',
        _ => return None,
    })
}

/// Labels as OCR spells them, one char for one char: `S` read as `5`, `I` as `(` or `1`.
fn norm_label(s: &str) -> String {
    s.chars()
        .map(|c| match c.to_ascii_uppercase() {
            '5' | '$' => 'S',
            '(' | ')' | '|' | '1' | 'l' => 'I',
            u => u,
        })
        .collect()
}

/// Edit distance, for short labels only.
fn edits(a: &[u8], b: &[u8]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, &x) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        for (j, &y) in b.iter().enumerate() {
            cur.push(
                (prev[j] + usize::from(x != y))
                    .min(prev[j + 1] + 1)
                    .min(cur[j] + 1),
            );
        }
        prev = cur;
    }
    prev[b.len()]
}

/// The text after `label` on the line that starts with it, allowing OCR one wrong, missing or
/// extra letter for labels of 3+ letters (`"ITS` is HITS, `MMEAN MS` is MEAN MS). A letter
/// right after the label means a longer word (MISS is not MISSES).
fn after<'a>(lines: &'a Lines, label: &str) -> Option<(&'a str, Rect)> {
    lines.iter().find_map(|(t, r)| {
        if !t.is_ascii() {
            return None;
        }
        // OCR adds stray marks before a line (a quote, a dot)
        let lead = t.len()
            - t.trim_start_matches(|c: char| !c.is_ascii_alphanumeric())
                .len();
        let n = norm_label(&t[lead..]);
        let slack = usize::from(label.len() >= 3);
        (label.len().saturating_sub(slack)..=label.len() + slack)
            .filter(|&k| k <= n.len())
            .filter(|&k| !n[k..].starts_with(|c: char| c.is_ascii_alphabetic()))
            .find(|&k| edits(&n.as_bytes()[..k], label.as_bytes()) <= slack)
            .map(|k| (&t[lead + k..], *r))
    })
}

/// Value text with digit look-alikes repaired; `None` if anything else is in it.
fn digits(rest: &str) -> Option<String> {
    let v: Option<String> = rest
        .trim_start_matches([':', ';', ' ', '.', '?', '\'', '!'])
        .chars()
        .filter(|c| !c.is_whitespace())
        .map(repair)
        .collect();
    v.filter(|v| !v.is_empty())
}

fn uint(lines: &Lines, label: &str) -> Option<u32> {
    digits(after(lines, label)?.0)?.parse().ok()
}

/// A 2-decimal value: as read when it has a dot and two decimals, `100` as 100 (a full
/// accuracy is shown without decimals), else with the dot put back where the format says it
/// goes (OCR drops it).
fn two_dp(digits: &str) -> Option<f32> {
    if let Some((_, f)) = digits.split_once('.')
        && f.len() == 2
        && f.chars().all(|c| c.is_ascii_digit())
    {
        return digits.parse().ok();
    }
    if digits.trim_start_matches('-') == "100" {
        return Some(100.0);
    }
    let neg = digits.starts_with('-');
    let d: String = digits.chars().filter(char::is_ascii_digit).collect();
    if d.len() < 3 || d.len() > 7 {
        return None;
    }
    let (i, f) = d.split_at(d.len() - 2);
    let v: f32 = format!("{i}.{f}").parse().ok()?;
    Some(if neg { -v } else { v })
}

/// One OCR pass over the stats panel.
pub fn stats(lines: &Lines) -> StatsRead {
    let pct = after(lines, "ACCURACY")
        .and_then(|(r, _)| digits(r))
        .and_then(|v| two_dp(v.trim_end_matches('%')))
        .filter(|v| (0.0..=100.0).contains(v));
    let mean = after(lines, "MEAN MS")
        .and_then(|(r, _)| {
            // the value ends where the trailing "ms" begins
            let end = r.to_uppercase().rfind('M').unwrap_or(r.len());
            digits(&r[..end])
        })
        .and_then(|v| two_dp(&v));
    StatsRead {
        accuracy: pct,
        highest_combo: uint(lines, "HIGHEST COMBO"),
        combo: uint(lines, "COMBO"),
        hits: uint(lines, "HITS"),
        misses: uint(lines, "MISSES"),
        mean_ms: mean,
        kps: uint(lines, "KPS"),
    }
}

/// One OCR pass over the judgment counts.
pub fn counts(lines: &Lines) -> Counts {
    Counts {
        sick: uint(lines, "SICK"),
        good: uint(lines, "GOOD"),
        ok: uint(lines, "OK"),
        bad: uint(lines, "BAD"),
        miss: uint(lines, "MISS"),
    }
}

/// The first word after `label`, repaired: the results screen can run a value into the next
/// text on its line ("TOTAL NOTES: 727 136430" is the notes and the final score).
fn first_uint(lines: &Lines, label: &str) -> Option<u32> {
    let rest = after(lines, label)?.0;
    let word = rest
        .trim_start_matches([':', ';', ' ', '.', '?', '\'', '!'])
        .split_whitespace()
        .next()?;
    digits(word)?.parse().ok()
}

/// The most common value, the first seen on a tie.
fn mode<T: PartialEq + Copy>(vals: impl Iterator<Item = Option<T>>) -> Option<T> {
    let mut t: Vec<(T, u32)> = vec![];
    for v in vals.flatten() {
        match t.iter_mut().find(|e| e.0 == v) {
            Some(e) => e.1 += 1,
            None => t.push((v, 1)),
        }
    }
    t.iter()
        .fold(None, |b: Option<&(T, u32)>, e| match b {
            Some(x) if x.1 >= e.1 => b,
            _ => Some(e),
        })
        .map(|e| e.0)
}

/// Final score on the results screen: 350 Sick + 200 Good + 100 Ok + 50 Bad - 30 Miss (exact on
/// all six screens of 2026-11-24, spec 13 5.3).
pub fn results_score(c: [u32; 5]) -> i64 {
    350 * i64::from(c[0]) + 200 * i64::from(c[1]) + 100 * i64::from(c[2]) + 50 * i64::from(c[3])
        - 30 * i64::from(c[4])
}

/// Equal, or the same length with one digit different (one OCR misread).
fn near(read: u32, want: i64) -> bool {
    let (a, b) = (read.to_string(), want.to_string());
    a.len() == b.len() && a.bytes().zip(b.bytes()).filter(|(x, y)| x != y).count() <= 1
}

/// The big number under "FINAL SCORE" (the nearest digit line below the label, within three
/// label heights), or the second word of the Total Notes line (OCR runs them together).
fn final_score(lines: &Lines) -> Option<u32> {
    let num = |t: &str| -> Option<u32> {
        let d = digits(t.trim())?;
        (d.len() >= 3 && d.chars().all(|c| c.is_ascii_digit()))
            .then(|| d.parse().ok())
            .flatten()
    };
    if let Some((_, lr)) = after(lines, "FINAL SCORE") {
        let below = lines
            .iter()
            .filter(|(_, r)| r.y > lr.y && r.y < lr.y + 4 * lr.h as i32 && (r.x - lr.x).abs() < 80)
            .min_by_key(|(_, r)| r.y)
            .and_then(|(t, _)| num(t));
        if below.is_some() {
            return below;
        }
    }
    let rest = after(lines, "TOTAL NOTES")?.0;
    num(rest
        .trim_start_matches([':', ';', ' ', '.', '?', '\'', '!'])
        .split_whitespace()
        .nth(1)?)
}

/// The most read size, negative if any pass read it with a minus: OCR drops a minus sign far
/// more often than it makes one up.
fn signed_mode(vals: impl Iterator<Item = f32>) -> Option<f32> {
    let v: Vec<f32> = vals.collect();
    let size = mode(v.iter().map(|x| Some(x.abs())))?;
    Some(if v.iter().any(|&x| x < 0.0 && x.abs() == size) {
        -size
    } else {
        size
    })
}

/// Stands for "not read" in a row of candidates.
const UNREAD: u32 = u32::MAX;

/// The results screen ("Stats from Last Song") from OCR passes (any frames, any crops). The
/// five counts, Total Notes and the final score are voted on together: a reading is kept only
/// when its equations hold (Total = the sum, score = `results_score`), with the most passes
/// behind it and no tie. One count may be unread or wrong when Total and the score are both
/// read: it is solved from Total and checked against the score. Total or score may be one
/// digit off (they are then corrected) when the other equation, or `hint` (the live HUD's
/// final counts), backs the counts. Accuracy outside 0..=100 and a combo above Total Notes are
/// dropped.
pub fn results(passes: &[Vec<(String, Rect)>], hint: Option<Counts>) -> Option<ResultsRead> {
    // A count may be taken as unread (the one solved from Total); Total and the score only
    // when no pass read them, so a reading that contradicts the counts is never set aside.
    let row = |f: &dyn Fn(&Lines) -> Option<u32>, skippable: bool| {
        let mut t = tally(passes.iter().map(|p| f(p)));
        if skippable || t.is_empty() {
            t.push((UNREAD, 0));
        }
        t
    };
    let rows = [
        row(&|l| first_uint(l, "SICK"), true),
        row(&|l| first_uint(l, "GOOD"), true),
        row(
            &|l| first_uint(l, "OK").or_else(|| first_uint(l, "OR")),
            true,
        ),
        row(&|l| first_uint(l, "BAD"), true),
        row(&|l| first_uint(l, "MISS"), true),
        row(&|l| first_uint(l, "TOTAL NOTES"), false),
        row(&final_score, false),
    ];
    let hint = hint.and_then(|h| Some([h.sick?, h.good?, h.ok?, h.bad?, h.miss?]));
    let solve = |v: &[u32]| -> Option<([u32; 5], u32, i64)> {
        let known = |x: u32| (x != UNREAD).then_some(x);
        let (t, p) = (known(v[5]), known(v[6]));
        let unread: Vec<usize> = (0..5).filter(|&i| v[i] == UNREAD).collect();
        let mut c = [v[0], v[1], v[2], v[3], v[4]];
        match unread[..] {
            [k] => {
                let rest: u32 = (0..5).filter(|&i| i != k).map(|i| c[i]).sum();
                c[k] = t?.checked_sub(rest)?;
                let score = results_score(c);
                near(p?, score).then_some((c, t?, score))
            }
            [] => {
                let (sum, score) = (c.iter().sum::<u32>(), results_score(c));
                let ok_t = t.is_none_or(|t| near(t, i64::from(sum)));
                let ok_p = p.is_none_or(|p| near(p, score));
                let backed = t == Some(sum)
                    || p.is_some_and(|p| i64::from(p) == score)
                    || (t.is_some() && p.is_some())
                    || hint == Some(c);
                (sum > 0 && ok_t && ok_p && backed).then_some((c, sum, score))
            }
            _ => None,
        }
    };
    let v = best_consistent(&rows, |v| solve(v).is_some())?;
    let (c, total, score) = solve(&v)?;
    let st: Vec<StatsRead> = passes.iter().map(|p| stats(p)).collect();
    Some(ResultsRead {
        counts: Counts {
            sick: Some(c[0]),
            good: Some(c[1]),
            ok: Some(c[2]),
            bad: Some(c[3]),
            miss: Some(c[4]),
        },
        total_notes: Some(total),
        score: u32::try_from(score).ok(),
        accuracy: mode(st.iter().map(|s| s.accuracy)),
        mean_ms: signed_mode(st.iter().filter_map(|s| s.mean_ms)),
        combo: mode(
            passes
                .iter()
                .map(|p| first_uint(p, "COMBO").filter(|&c| c <= total)),
        ),
        highest_combo: mode(
            passes
                .iter()
                .map(|p| first_uint(p, "HIGHEST COMBO").filter(|&c| c <= total)),
        ),
    })
}

/// The whole results panel (both columns and the score under its label), from the lines of
/// one full-frame pass: later passes read only this crop, which is faster and sharper.
pub fn results_area(lines: &Lines) -> Option<Rect> {
    let rs: Vec<Rect> = [
        "SICK",
        "GOOD",
        "BAD",
        "MISS",
        "TOTAL NOTES",
        "ACCURACY",
        "MEAN MS",
        "FINAL SCORE",
    ]
    .iter()
    .filter_map(|l| after(lines, l).map(|(_, r)| r))
    .collect();
    if rs.len() < 3 {
        return None;
    }
    let mut u = union(&rs)?;
    // the score digits sit under their label, about three label heights tall
    if let Some((_, lr)) = after(lines, "FINAL SCORE") {
        let bottom = (lr.y + 4 * lr.h as i32).max(u.y + u.h as i32);
        u.h = (bottom - u.y) as u32;
    }
    Some(padded(u, 40))
}

/// The value every pass read alike (for values no arithmetic can check).
fn all_agree<T: PartialEq + Copy>(vals: impl Iterator<Item = Option<T>>) -> Option<T> {
    let v: Vec<Option<T>> = vals.collect();
    let first = (*v.first()?)?;
    (v.len() >= 2 && v.iter().all(|x| *x == Some(first))).then_some(first)
}

/// Every distinct value any pass read, with how many passes read it.
fn tally(vals: impl Iterator<Item = Option<u32>>) -> Vec<(u32, u32)> {
    let mut t: Vec<(u32, u32)> = vec![];
    for v in vals.flatten() {
        match t.iter_mut().find(|e| e.0 == v) {
            Some(e) => e.1 += 1,
            None => t.push((v, 1)),
        }
    }
    t
}

/// The combination of candidates (one per row) that satisfies `ok`, with the most passes
/// behind it; `None` when there is none or two tie (then nothing is trusted).
fn best_consistent(rows: &[Vec<(u32, u32)>], ok: impl Fn(&[u32]) -> bool) -> Option<Vec<u32>> {
    let mut best: Option<(u32, Vec<u32>)> = None;
    let mut tie = false;
    let mut pick = vec![0usize; rows.len()];
    if rows.iter().any(Vec::is_empty) {
        return None;
    }
    loop {
        let vals: Vec<u32> = pick.iter().zip(rows).map(|(&i, r)| r[i].0).collect();
        if ok(&vals) {
            let score = pick.iter().zip(rows).map(|(&i, r)| r[i].1).sum::<u32>();
            match &best {
                Some((s, _)) if score < *s => {}
                Some((s, _)) if score == *s => tie = true,
                _ => {
                    best = Some((score, vals));
                    tie = false;
                }
            }
        }
        // next combination (rows have at most a handful of candidates each)
        let mut k = 0;
        loop {
            if k == rows.len() {
                return best.filter(|_| !tie).map(|b| b.1);
            }
            pick[k] += 1;
            if pick[k] < rows[k].len() {
                break;
            }
            pick[k] = 0;
            k += 1;
        }
    }
}

/// Values that are safe to teach the glyph reader. Hits and the four hit counts are trusted
/// together when they add up, Misses and Miss when they are equal (the best-supported such
/// reading, unique); the rest when every pass agrees. Everything else is `None`.
pub fn trusted(stats_passes: &[StatsRead], count_passes: &[Counts]) -> (StatsRead, Counts) {
    let st = |f: fn(&StatsRead) -> Option<u32>| tally(stats_passes.iter().map(f));
    let cn = |f: fn(&Counts) -> Option<u32>| tally(count_passes.iter().map(f));
    let hits = best_consistent(
        &[
            st(|p| p.hits),
            cn(|p| p.sick),
            cn(|p| p.good),
            cn(|p| p.ok),
            cn(|p| p.bad),
        ],
        |v| v[0] == v[1] + v[2] + v[3] + v[4],
    );
    let miss = best_consistent(&[st(|p| p.misses), cn(|p| p.miss)], |v| v[0] == v[1]);
    let same = |f: fn(&StatsRead) -> Option<u32>| all_agree(stats_passes.iter().map(f));
    let (combo, highest) = match (same(|p| p.combo), same(|p| p.highest_combo)) {
        (Some(c), Some(h)) if c > h => (None, None),
        x => x,
    };
    let h = |i: usize| hits.as_ref().map(|v| v[i]);
    (
        StatsRead {
            accuracy: all_agree(stats_passes.iter().map(|p| p.accuracy)),
            highest_combo: highest,
            combo,
            hits: h(0),
            misses: miss.as_ref().map(|v| v[0]),
            mean_ms: all_agree(stats_passes.iter().map(|p| p.mean_ms)),
            kps: same(|p| p.kps),
        },
        Counts {
            sick: h(1),
            good: h(2),
            ok: h(3),
            bad: h(4),
            miss: miss.map(|v| v[1]),
        },
    )
}

/// Where the HUD elements are, from full-frame OCR lines (any side of the screen).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Found {
    pub stats: Option<Rect>,
    pub counts: Option<Rect>,
    pub ms: Option<Rect>,
    pub title: Option<Rect>,
}

fn union(rs: &[Rect]) -> Option<Rect> {
    let x0 = rs.iter().map(|r| r.x).min()?;
    let y0 = rs.iter().map(|r| r.y).min()?;
    let x1 = rs.iter().map(|r| r.x + r.w as i32).max()?;
    let y1 = rs.iter().map(|r| r.y + r.h as i32).max()?;
    Some(Rect::new(x0, y0, (x1 - x0) as u32, (y1 - y0) as u32))
}

/// `r` padded by `p` on each side, so glyph reads are not cut.
fn padded(r: Rect, p: u32) -> Rect {
    Rect::new(r.x - p as i32, r.y - p as i32, r.w + 2 * p, r.h + 2 * p)
}

/// Is `t` an ms readout, `-?n.nn ms`, after repair?
pub fn ms_value(t: &str) -> Option<f32> {
    let up = t.to_uppercase();
    let end = up.rfind("M5").or_else(|| up.rfind("MS"))?;
    let v: Option<String> = t[..end]
        .chars()
        .filter(|c| !c.is_whitespace())
        .map(repair)
        .collect();
    let v = v?;
    // a readout always shows the dot; demanding it keeps stats rows ("Mean MS: 0.00 ms") apart
    (v.contains('.') && v.split('.').nth(1).is_some_and(|d| d.len() == 2))
        .then(|| v.parse().ok())
        .flatten()
}

pub fn locate(lines: &Lines, playfield: Option<(i32, i32)>, stream_w: u32) -> Found {
    let rows = |labels: &[&str]| {
        // Every line that reads as one of the labels (the FPS counter can read as KPS, so not
        // just the first), with the label it matched.
        let mut rs: Vec<(Rect, usize)> = lines
            .iter()
            .filter_map(|line| {
                let one = std::slice::from_ref(line);
                labels
                    .iter()
                    .position(|l| after(one, l).is_some())
                    .map(|i| (line.1, i))
            })
            .collect();
        rs.sort_by_key(|r| r.0.y);
        // One panel: the longest run of rows each within 5 row heights of the one above (two
        // rows OCR could not read leave a gap).
        let (mut best, mut start) = ((0, 0), 0);
        for i in 1..=rs.len() {
            if i == rs.len() || rs[i].0.y - rs[i - 1].0.y > 5 * rs[i - 1].0.h as i32 {
                if i - start > best.1 - best.0 {
                    best = (start, i);
                }
                start = i;
            }
        }
        let mut run: Vec<(Rect, usize)> = rs[best.0..best.1].to_vec();
        // and left-aligned: a judgment popup at the same height ("GOOD!") is not a row
        if let Some(h) = run.first().map(|r| r.0.h as i32) {
            let mut xs: Vec<i32> = run.iter().map(|r| r.0.x).collect();
            xs.sort_unstable();
            let mid = xs[xs.len() / 2];
            run.retain(|r| (r.0.x - mid).abs() <= 3 * h);
        }
        let mut seen: Vec<usize> = run.iter().map(|r| r.1).collect();
        seen.sort_unstable();
        seen.dedup();
        let rects: Vec<Rect> = run.iter().map(|r| r.0).collect();
        // most rows must be seen, else it is some other text
        let row_h = rects.first().map_or(0, |r| r.h);
        (seen.len() * 3 >= labels.len() * 2)
            .then(|| union(&rects).map(|u| padded(u, row_h / 2)))
            .flatten()
    };
    let stats = rows(&[
        "ACCURACY",
        "HIGHEST COMBO",
        "HITS",
        "MISSES",
        "MEAN MS",
        "KPS",
    ]);
    let counts = rows(&["SICK", "GOOD", "OK", "BAD", "MISS"]);
    // Where our readout is: a line in our playfield ending in "ms" with digits before it. Its
    // thin font often defeats OCR's digits ("731MS" for 78.71), but not its position.
    let ms = lines
        .iter()
        .filter(|(t, r)| {
            let c = r.x + r.w as i32 / 2;
            let up = t.to_uppercase();
            !up.contains("MEAN")
                && (up.trim_end().ends_with("MS") || up.trim_end().ends_with("M5"))
                && t.chars().filter(char::is_ascii_digit).count() >= 2
                && playfield.is_some_and(|(a, b)| (a..=b).contains(&c))
        })
        .map(|(_, r)| padded(*r, r.h / 2))
        .next();
    let mid = stream_w as i32 / 2;
    let title = lines
        .iter()
        .filter(|(t, r)| {
            let c = r.x + r.w as i32 / 2;
            t.len() >= 6 && (c - mid).abs() < stream_w as i32 / 8
        })
        .min_by_key(|(_, r)| r.y)
        .map(|(_, r)| *r);
    Found {
        stats,
        counts,
        ms,
        title,
    }
}

/// Song title and difficulty from the title line: "Pack - Song (Difficulty)".
pub fn title(t: &str) -> (String, String) {
    let t = t.trim();
    match (t.rfind('('), t.rfind(')')) {
        (Some(a), Some(b)) if b > a => (t[..a].trim().to_string(), t[a + 1..b].trim().to_string()),
        _ => (t.to_string(), String::new()),
    }
}

/// Song rate from a title's "(2.00x)" (OCR often drops the dot: "(200X)"). The game's ms are
/// in song time at that rate (G15). `None`: no rate shown (1.00x).
pub fn rate(t: &str) -> Option<f32> {
    t.rsplit('(').find_map(|seg| {
        let inner = seg.split(')').next()?.trim();
        let num = inner.strip_suffix(['x', 'X'])?.trim().replace(',', ".");
        let v: f32 = if num.contains('.') {
            num.parse().ok()?
        } else {
            let d: f32 = num.parse().ok()?;
            match num.len() {
                3 => d / 100.0,
                2 => d / 10.0,
                1 => d,
                _ => return None,
            }
        };
        (0.25..=4.0).contains(&v).then_some(v)
    })
}

/// The ms of the hit that took Hits from `prev_hits` to `hits = prev_hits + 1`, from the
/// stats panel's Mean MS before and after (shown to 2 decimals), and how far off it can be
/// from that rounding. `None` unless exactly one hit was added.
pub fn hit_ms_from_mean(
    prev_mean: f32,
    prev_hits: u32,
    mean: f32,
    hits: u32,
) -> Option<(f32, f32)> {
    (hits == prev_hits + 1).then(|| {
        let v = hits as f32 * mean - prev_hits as f32 * prev_mean;
        (v, 0.005 * (2 * hits - 1) as f32 + 0.001)
    })
}

/// The readout value to trust: the one candidate (OCR passes, glyph read) within `tol` of
/// `expected`. Several different values that fit, or none: `None`.
pub fn confirm_ms(cands: &[f32], expected: f32, tol: f32) -> Option<f32> {
    let mut fit = cands
        .iter()
        .copied()
        .filter(|c| (c - expected).abs() <= tol);
    let first = fit.next()?;
    fit.all(|c| (c - first).abs() < 0.005).then_some(first)
}

/// Score regions `[left, right]`: the lowest text line in the outer quarter of each side
/// gives the row; the region runs from the dark gap after the avatar at the screen edge (OCR
/// reads the avatar as digits) inwards to the quarter line, as scores grow.
pub fn score_regions(lines: &Lines, f: &FrameView) -> [Option<Rect>; 2] {
    let stream = [f.region.w, f.region.h];
    [true, false].map(|left| score_region(lines, f, stream, left))
}

/// The bottom corner of a `stream`-sized screen where that side's score is (`score_region`
/// reads only inside it).
pub fn score_corner(stream: [u32; 2], left: bool) -> Rect {
    let (w, h) = (stream[0] as i32, stream[1] as i32);
    Rect::new(
        if left { 0 } else { w * 3 / 4 },
        h * 3 / 4,
        (w / 4) as u32,
        (h / 4) as u32,
    )
}

/// One side of `score_regions`; `f` must hold that side's `score_corner`.
pub fn score_region(lines: &Lines, f: &FrameView, stream: [u32; 2], left: bool) -> Option<Rect> {
    let (w, h) = (stream[0] as i32, stream[1] as i32);
    let side = |left: bool| {
        let (_, r) = lines
            .iter()
            .filter(|(t, r)| {
                let c = r.x + r.w as i32 / 2;
                r.y > h * 4 / 5
                    && (if left { c < w / 4 } else { c > w * 3 / 4 })
                    && t.chars().any(|c| repair(c).is_some())
                    // score digits are about 1/27 of the height; smaller lines are noise
                    && r.h as i32 >= h / 36
            })
            .max_by_key(|(_, r)| r.y)?;
        let pad = r.h as i32 / 4;
        let band = Rect::new(0, r.y - pad, 1, r.h + 2 * pad as u32)
            .intersect(&Rect::new(0, f.region.y, 1, f.region.h))?;
        let lit = |x: i32| lit_column(f, band, x);
        // from the edge: the avatar, a gap of at least a fifth of the text height, then the
        // score's first lit column (digits are at most a few px apart)
        let gap = (r.h as i32 / 5).max(6);
        let xs: Box<dyn Iterator<Item = i32>> = if left {
            Box::new(0..w / 4)
        } else {
            Box::new((w * 3 / 4..w).rev())
        };
        let (mut seen, mut dark, mut edge) = (false, 0, None);
        for x in xs {
            if !lit(x) {
                dark += 1;
            } else if seen && dark >= gap {
                edge = Some(if left { x - 4 } else { x + 5 });
                break;
            } else {
                (seen, dark) = (true, 0);
            }
        }
        let e = edge?;
        let (x0, x1) = if left { (e, w / 4) } else { (w * 3 / 4, e) };
        Some(Rect::new(x0, band.y, (x1 - x0).max(1) as u32, band.h))
    };
    side(left)
}

/// Column `x` of `band` holds bright text (2+ pixels above luma 100).
fn lit_column(f: &FrameView, band: Rect, x: i32) -> bool {
    (band.y..band.y + band.h as i32)
        .filter(|&y| {
            let (cr, cg, cb) = f.px(x, y);
            (cr as u32 + 2 * cg as u32 + cb as u32) / 4 > 100
        })
        .count()
        >= 2
}

/// Does `v` fit the lit text on the score crop `f`? Its length must match the text's width (a
/// glyph is about 0.75 text heights wide, and a third of the crop is padding), so a read that
/// drops or adds characters (`-19543` as `9543`) fails; and its sign must match the first
/// glyph: a minus is a flat stroke, under 0.4 text heights tall, where digits are nearly full
/// height (OCR merges `-1` into `2`: `-10230` as `210230`).
pub fn score_fits(f: &FrameView, v: i32) -> bool {
    let r = f.region;
    let text_h = r.h as f32 / 1.5;
    let mut xs = (r.x..r.x + r.w as i32).filter(|&x| lit_column(f, r, x));
    let Some(a) = xs.next() else { return false };
    let b = xs.next_back().unwrap_or(a);
    let est = (b - a + 1) as f32 / (0.75 * text_h);
    let lit_rows: Vec<i32> = (r.y..r.y + r.h as i32)
        .filter(|&y| {
            (a..(a + (0.3 * text_h) as i32).min(b + 1)).any(|x| {
                let (cr, cg, cb) = f.px(x, y);
                (cr as u32 + 2 * cg as u32 + cb as u32) / 4 > 100
            })
        })
        .collect();
    let tall = lit_rows
        .last()
        .zip(lit_rows.first())
        .map_or(0, |(z, a)| z - a + 1);
    let minus = (tall as f32) < 0.4 * text_h;
    (v.to_string().len() as f32 - est).abs() <= 1.0 && minus == (v < 0)
}

/// A score as OCR read it: the widest line (OCR adds small stray marks) is one signed integer,
/// look-alikes repaired.
pub fn score_value(lines: &Lines) -> Option<i32> {
    let (t, _) = lines.iter().max_by_key(|(_, r)| r.w)?;
    let v = digits(t.trim())?;
    let ok = v
        .char_indices()
        .all(|(i, c)| c.is_ascii_digit() || (c == '-' && i == 0));
    ok.then(|| v.parse().ok()).flatten()
}

/// OCR's two ways of losing the minus before the game font's `1` (the minus is seen in the
/// pixels, so a positive read is wrong): the minus reads as `2` (`-10230` as `210230`), or the
/// `-1` vanishes (`-19543` as `9543`). Candidates still have to fit the crop and agree with
/// another pass.
pub fn minus_one_repairs(v: i32) -> impl Iterator<Item = i32> {
    let s = v.to_string();
    let merged = s.strip_prefix('2').map(|r| format!("-{r}"));
    let dropped = (v >= 0).then(|| format!("-1{s}"));
    [merged, dropped]
        .into_iter()
        .flatten()
        .filter_map(|t| t.parse().ok())
}

/// The score to trust: a value at least two OCR passes agree on.
pub fn confirm_score(cands: &[i32]) -> Option<i32> {
    cands
        .iter()
        .copied()
        .find(|v| cands.iter().filter(|c| *c == v).count() >= 2)
}

/// Can a score have gone from `prev` to `v` in `secs`? Two OCR passes can agree on a misread
/// (`-10230` as `210230`: the minus and the 1 merge into a 2), so a read must also follow on
/// from the last trusted one. 10k points a second is above any chart's rate.
pub fn plausible_score(prev: i32, v: i32, secs: f32) -> bool {
    ((v as i64 - prev as i64).abs() as f32) <= 1000.0 + 10_000.0 * secs
}

/// Mean colour of the saturated bright pixels in `r` (coloured text; the white of values and
/// the dark outline are left out). `None` with too few such pixels.
pub fn text_colour(f: &arrowhead_core::frame::FrameView, r: Rect) -> Option<[f32; 3]> {
    let r = f.region.intersect(&r)?;
    let (mut sum, mut n) = ([0f32; 3], 0u32);
    for y in r.y..r.y + r.h as i32 {
        for x in r.x..r.x + r.w as i32 {
            let (cr, cg, cb) = f.px(x, y);
            let (hi, lo) = (cr.max(cg).max(cb), cr.min(cg).min(cb));
            if hi > 140 && hi - lo > 60 {
                sum[0] += cr as f32;
                sum[1] += cg as f32;
                sum[2] += cb as f32;
                n += 1;
            }
        }
    }
    (n >= 30).then(|| sum.map(|v| v / n as f32))
}

/// The five count-row label colours (Sick..Miss): the stack split into five equal bands, left
/// half of each (the label; values can be white).
pub fn label_colours(f: &arrowhead_core::frame::FrameView, counts: Rect) -> Option<[[f32; 3]; 5]> {
    let band = counts.h / 5;
    let mut out = [[0f32; 3]; 5];
    for (i, o) in out.iter_mut().enumerate() {
        let r = Rect::new(
            counts.x,
            counts.y + (i as u32 * band) as i32,
            counts.w / 2,
            band,
        );
        *o = text_colour(f, r)?;
    }
    Some(out)
}

/// Judgment index (0 Sick .. 4 Miss) of a readout drawn in the judgment's colour: the
/// count-row label colour nearest to `rgb` (each learned from the counts panel). `None`
/// when no label is clearly nearest.
pub fn judgment_by_colour(rgb: [f32; 3], labels: &[[f32; 3]; 5]) -> Option<usize> {
    let d = |a: &[f32; 3]| (0..3).map(|i| (a[i] - rgb[i]).powi(2)).sum::<f32>().sqrt();
    let mut ds: Vec<(f32, usize)> = labels.iter().enumerate().map(|(i, l)| (d(l), i)).collect();
    ds.sort_by(|a, b| a.0.total_cmp(&b.0));
    (ds[0].0 < 80.0 && ds[1].0 > 1.5 * ds[0].0).then_some(ds[0].1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn song_rate_from_title() {
        assert_eq!(rate("VS CAMELLIA GHOST (VIP) (MANIAC) (200X)"), Some(2.0));
        assert_eq!(rate("Ghost (VIP) (Maniac) (2.00x)"), Some(2.0));
        assert_eq!(rate("Song (1.5x)"), Some(1.5));
        assert_eq!(rate("Song (Hard)"), None);
        assert_eq!(rate("Song (VIP)"), None);
        assert_eq!(rate("Song (900x)"), None);
    }

    #[test]
    fn score_helpers() {
        let l = |t: &str, w| (t.to_string(), Rect::new(0, 0, w, 10));
        // the widest line, repaired; a stray mark is ignored
        assert_eq!(score_value(&[l("T", 4), l("-3998", 40)]), Some(-3998));
        assert_eq!(score_value(&[l("12-3", 40)]), None);
        let reps: Vec<i32> = minus_one_repairs(210230).collect();
        assert_eq!(reps, vec![-10230, -1210230]);
        assert_eq!(minus_one_repairs(9543).collect::<Vec<_>>(), vec![-19543]);
        assert_eq!(minus_one_repairs(-5).count(), 0);
        assert_eq!(confirm_score(&[5, 7, 7]), Some(7));
        assert_eq!(confirm_score(&[5, 7]), None);
        assert!(plausible_score(0, 900, 0.0));
        assert!(!plausible_score(0, 210230, 2.0));
        assert!(plausible_score(-10000, -10230, 0.1));
    }

    fn l(t: &str, y: i32) -> (String, Rect) {
        (t.to_string(), Rect::new(10, y, 200, 20))
    }

    // Real ocrs output on the user's frame (s88): plain 2x, 3x, and threshold 2x passes.
    fn passes() -> Vec<Vec<(String, Rect)>> {
        let mk = |rows: [&str; 7]| {
            rows.iter()
                .enumerate()
                .map(|(i, t)| l(t, i as i32 * 40))
                .collect()
        };
        vec![
            mk([
                "ACCURACY: S244%",
                "HIGHEST COMBO: ??",
                "COMBO: Y",
                "H(T5: 2)9",
                "MISSES: 70",
                "MEAN M5: 77.42 M5",
                "KPS: 8",
            ]),
            mk([
                "ACCURACY: 5244%",
                "HIGHEST COMBO: 77",
                "COMBO: 4",
                "H(T5: 2)9",
                "MISSES: ?0",
                "MEAN M5: 27.42 M5",
                "KPS: 8",
            ]),
            mk([
                "ACCURACY: $24Y%",
                "HIGHEST COMBO: 77",
                "COMBO: 4",
                "HIT5: 219",
                "MISSES: 170",
                "MEAN M5: 27,42 M5",
                "KP5: 18",
            ]),
        ]
    }

    fn count_passes() -> Vec<Vec<(String, Rect)>> {
        let mk = |rows: [&str; 5]| {
            rows.iter()
                .enumerate()
                .map(|(i, t)| l(t, i as i32 * 40))
                .collect()
        };
        vec![
            mk(["SICK 8'5", "GOOD: GI", "OK: 47", "BAD: 26", "MISS:70"]),
            mk(["SICK 85", "GOOD: 61", "OK? 4?", "BAD: 26", "MIS: 170"]),
            mk(["SICK: 85", "GOOD: GN", "OR 47", "BAD: 26", "MI55: 170"]),
        ]
    }

    fn results(p: &[Vec<(String, Rect)>]) -> Option<ResultsRead> {
        super::results(p, None)
    }

    fn lines(rows: &[&str]) -> Vec<(String, Rect)> {
        rows.iter()
            .enumerate()
            .map(|(i, t)| l(t, i as i32 * 40))
            .collect()
    }

    #[test]
    fn reads_the_results_panel() {
        // real ocrs output on results4 (plain 2x): Miss 61 read as 6 and Total 688 as 633 add
        // up by chance, but the score for those counts would be 137170, not 135320: rejected
        let mut rows = vec![
            "STATS FROM LAST SONG",
            "SICk: 243",
            "GOOD: 180",
            "Ok: 122",
            "BAD: 82",
            "MISS: 6",
            "TOTAL NOTES: 633",
            "ACCURACY: 84.93%",
            "MEAN M5: 48.40 M5",
            "COMBO: 2",
            "HIGHEST COMBO: 47",
            "FINAL SCORE",
            "135320",
        ];
        assert_eq!(results(&[lines(&rows)]), None);
        // the true Miss and Total: the score read one digit off (135320 for 135520) is fixed
        rows[5] = "MISS: 61";
        rows[6] = "TOTAL NOTES: 688";
        let r = results(&[lines(&rows)]).unwrap();
        assert_eq!(r.counts.sick, Some(243));
        assert_eq!(r.counts.ok, Some(122));
        assert_eq!(r.counts.miss, Some(61));
        assert_eq!(r.total_notes, Some(688));
        assert_eq!(r.score, Some(135_520));
        assert_eq!(r.accuracy, Some(84.93));
        assert_eq!(r.mean_ms, Some(48.4));
        assert_eq!((r.combo, r.highest_combo), (Some(2), Some(47)));
        // a quote before a label, O for 0, R for K, the final score run onto the total line
        let r = results(&[lines(&[
            "SICk: 243",
            "\"GO0D:177",
            "OR: 123",
            "BAD: 115",
            "MI5S: 69",
            "TOTAL NOTES: 727 136430",
            "ACCURACY: 82 98%",
        ])])
        .unwrap();
        assert_eq!(r.counts.good, Some(177));
        assert_eq!(r.counts.ok, Some(123));
        assert_eq!(r.total_notes, Some(727));
        assert_eq!(r.accuracy, Some(82.98));
    }

    #[test]
    fn results_that_do_not_add_up_are_rejected() {
        // "BAD: S" (a 5 for the real 115): the counts miss Total Notes
        let bad = lines(&[
            "SICk: 243",
            "GOOD: 177",
            "OR: 123",
            "BAD: S",
            "MI5S: 69",
            "TOTAL NOTES: 727",
        ]);
        assert_eq!(results(&[bad]), None);
        // a count that does not parse at all
        let bad = lines(&[
            "SICk: 190",
            "GOOD: 112",
            "OR: 74",
            "BAD: S?",
            "MI5S: S?",
            "TOTAL NOTES: 488",
        ]);
        assert_eq!(results(&[bad]), None);
        // no Total Notes line
        let bad = lines(&["SICk: 1", "GOOD: 1", "OK: 1", "BAD: 1", "MISS: 1"]);
        assert_eq!(results(&[bad]), None);
    }

    #[test]
    fn results_vote_across_passes() {
        // real ocrs output on the full frame and on the counts column (results4): no single
        // pass has Miss right (6, 60, 61), together with Total Notes 688 they settle it
        let full = lines(&[
            "SICk: 243",
            "GOOD: 180",
            "Ok: 122",
            "BAD: 82",
            "MISS: 6",
            "TOTAL NOTES: 688",
            "ACCURACY: 84.93%",
        ]);
        let col = lines(&[
            "SICK: 243",
            "GOOD: 180",
            "Ok: 122",
            "BAD: 82",
            "MI55: 6)",
            "TOTAL NOTES: 688",
        ]);
        assert_eq!(results(std::slice::from_ref(&full)), None);
        let r = results(&[full.clone(), col]).unwrap();
        assert_eq!(r.counts.miss, Some(61));
        assert_eq!(r.accuracy, Some(84.93));
        // C is a 6 in the game font (results3: Bad 56, Miss 56)
        let r = results(&[lines(&[
            "SICk: 190",
            "GOOD: 112",
            "OR: 74",
            "BAD: 56",
            "MISS: SC",
            "TOTAL NOTES: 488",
        ])])
        .unwrap();
        assert_eq!((r.counts.bad, r.counts.miss), (Some(56), Some(56)));
        assert!(results_area(&full).is_some());
    }

    #[test]
    fn results_checked_and_fixed_by_the_score() {
        // Deathmatch (2026-11-24): score read 240804 for 240800, Total exact: fixed
        let mut rows = vec![
            "SICK: 688",
            "GOOD: 0",
            "OK: 0",
            "BAD: 0",
            "MISS: 0",
            "TOTAL NOTES: 688",
            "ACCURACY: 100%",
            "FINAL SCORE",
            "240804",
        ];
        let r = results(&[lines(&rows)]).unwrap();
        assert_eq!((r.score, r.accuracy), (Some(240_800), Some(100.0)));
        // Total read 788, score exact: fixed
        rows[5] = "TOTAL NOTES: 788";
        rows[8] = "240800";
        assert_eq!(results(&[lines(&rows)]).unwrap().total_notes, Some(688));
        // both one digit off: the two weak checks together are enough
        rows[8] = "240804";
        assert_eq!(results(&[lines(&rows)]).unwrap().total_notes, Some(688));
        // Gospel: Ok unreadable, solved from Total and checked by the score
        let r = results(&[lines(&[
            "SICK: 688",
            "GOOD: 0",
            "OK: ?",
            "BAD: 2",
            "MISS: 21",
            "TOTAL NOTES: 727",
            "FINAL SCORE",
            "241870",
        ])])
        .unwrap();
        assert_eq!(r.counts.ok, Some(16));
        // Pico (real OCR lines): no Total line and the score read 242310 for 247310; only
        // the live HUD's final counts can back it
        let pico = lines(&[
            "SIck. 709",
            "GOOD: O",
            "OK: 0",
            "SAD: O",
            "1I55 28",
            "ACCURACY: 96.20% ",
            "MEAN MS: -4.21 MS",
            "SINAL SCORE",
            "242310",
        ]);
        assert_eq!(results(std::slice::from_ref(&pico)), None);
        let live = Counts {
            sick: Some(709),
            good: Some(0),
            ok: Some(0),
            bad: Some(0),
            miss: Some(28),
        };
        let r = super::results(&[pico], Some(live)).unwrap();
        assert_eq!((r.total_notes, r.score), (Some(737), Some(247_310)));
        assert_eq!((r.accuracy, r.mean_ms), (Some(96.2), Some(-4.21)));
        assert_eq!(results_score([4696, 15, 48, 9, 193]), 1_646_060);
    }

    #[test]
    fn mean_keeps_a_minus_any_pass_saw() {
        assert_eq!(signed_mode([3.39, -3.39, 3.39].into_iter()), Some(-3.39));
        assert_eq!(signed_mode([5.39, 5.39].into_iter()), Some(5.39));
        assert_eq!(signed_mode(std::iter::empty()), None);
    }

    #[test]
    fn results_drop_an_implausible_accuracy() {
        let r = results(&[lines(&[
            "SICk: 1",
            "GOOD: 1",
            "OK: 1",
            "BAD: 1",
            "MISS: 1",
            "TOTAL NOTES: 5",
            "ACCURACY: 84.482%",
            "HIGHEST COMBO: 99",
        ])])
        .unwrap();
        assert_eq!(r.accuracy, None, "844.82 is out of range");
        assert_eq!(r.highest_combo, None, "more than the notes");
    }

    #[test]
    fn repairs_the_game_font() {
        let p = passes();
        let s = stats(&p[2]);
        assert_eq!(s.hits, Some(219));
        assert_eq!(s.misses, Some(170));
        assert_eq!(s.kps, Some(18));
        assert_eq!(s.accuracy, Some(52.44));
        assert_eq!(stats(&p[0]).accuracy, Some(52.44));
        assert_eq!(stats(&p[0]).hits, Some(219), "H(T5: 2)9 is 219");
        assert_eq!(counts(&count_passes()[0]).good, Some(61), "GI is 61");
    }

    #[test]
    fn votes_and_cross_checks() {
        let sp: Vec<StatsRead> = passes().iter().map(|p| stats(p)).collect();
        let cp: Vec<Counts> = count_passes().iter().map(|p| counts(p)).collect();
        let (s, c) = trusted(&sp, &cp);
        assert_eq!(s.accuracy, Some(52.44));
        // Combo reads Y, 4, 4: Y is a 4 in this font, so all agree. Highest Combo read ?? once
        // and no arithmetic checks it, so it waits for a cleaner moment.
        assert_eq!((s.highest_combo, s.combo), (None, Some(4)));
        assert_eq!(s.hits, Some(219));
        assert_eq!(
            s.mean_ms, None,
            "two passes agree on 27.42, one says 77.42: not trusted"
        );
        assert_eq!(
            (c.sick, c.good, c.ok, c.bad),
            (Some(85), Some(61), Some(47), Some(26))
        );
        // Misses read 70 / 170; Miss reads 70, 170 ("MIS: 170", one letter short) and 170:
        // 170 = 170 has the most passes behind it.
        assert_eq!((s.misses, c.miss), (Some(170), Some(170)));
    }

    #[test]
    fn hit_ms_checks_the_readout() {
        // 218 hits at mean 77.41 -> 219 at 77.42 gives about 79.6 +/- 2.2 ms.
        let (v, tol) = hit_ms_from_mean(77.41, 218, 77.42, 219).unwrap();
        assert!(
            (v - 79.6).abs() < 0.1 && (tol - 2.186).abs() < 0.01,
            "{v} {tol}"
        );
        assert_eq!(confirm_ms(&[78.71, 72.71], v, tol), Some(78.71));
        assert_eq!(
            confirm_ms(&[72.71], v, tol),
            None,
            "OCR's 72.71 is not that hit"
        );
        assert_eq!(
            hit_ms_from_mean(77.4, 218, 77.42, 220),
            None,
            "two hits: no single value"
        );
        // early in a song the bound is tight: hit 2 at mean 10.00 after hit 1 at 12.34
        let (v2, t2) = hit_ms_from_mean(12.34, 1, 10.0, 2).unwrap();
        assert!((v2 - 7.66).abs() < 1e-3 && t2 < 0.02);
    }

    #[test]
    fn judgment_from_readout_colour() {
        // label colours as seen on the counts panel: magenta, green, yellow, orange, red
        let labels = [
            [230.0, 90.0, 230.0],
            [90.0, 230.0, 80.0],
            [240.0, 220.0, 70.0],
            [240.0, 160.0, 60.0],
            [240.0, 80.0, 80.0],
        ];
        assert_eq!(
            judgment_by_colour([100.0, 225.0, 95.0], &labels),
            Some(1),
            "green readout: Good"
        );
        assert_eq!(
            judgment_by_colour([128.0, 128.0, 128.0], &labels),
            None,
            "grey: unsure"
        );
    }

    #[test]
    fn counts_that_do_not_add_up_are_not_trusted() {
        let sp = vec![
            StatsRead {
                hits: Some(219),
                ..Default::default()
            };
            2
        ];
        let bad = Counts {
            sick: Some(85),
            good: Some(61),
            ok: Some(47),
            bad: Some(2),
            miss: Some(170),
        };
        let (s, c) = trusted(&sp, &[bad, bad]);
        assert_eq!(s.hits, None);
        assert_eq!(c.bad, None);
    }

    #[test]
    fn labels_survive_ocr_damage() {
        // full-frame OCR lines from the user's frame (s88)
        let lines: Vec<(String, Rect)> = [
            "ACCURACY: 52 ?4%",
            "HIGHEST COMBO: 27",
            "OMBO:",
            "\"ITS: 219",
            "MISSES: 170",
            "MMEAN MS: 2742 MS",
            "P5: 18",
            "SICK 85",
            "GOOD!: 61",
            "OR? 47",
            "BAD: 24",
            "M155:70",
        ]
        .iter()
        .enumerate()
        .map(|(i, t)| l(t, i as i32 * 40))
        .collect();
        let s = stats(&lines);
        assert_eq!((s.hits, s.misses, s.kps), (Some(219), Some(170), Some(18)));
        let c = counts(&lines);
        assert_eq!(
            (c.sick, c.good, c.bad, c.miss),
            (Some(85), Some(61), Some(24), Some(70))
        );
        assert_eq!(c.ok, None, "OK is 2 letters: no slack, OR is not OK");
        assert!(
            after(&lines, "MISS").is_some_and(|(_, r)| r.y == 11 * 40),
            "not the MISSES row"
        );
    }

    #[test]
    fn finds_elements_and_title() {
        let lines = vec![
            (
                "ONLINE V'S CHALLENGES - ACCELERANT (INSANE)".to_string(),
                Rect::new(1080, 4, 400, 20),
            ),
            ("ACCURACY: 52.44%".to_string(), Rect::new(10, 610, 240, 24)),
            ("HIGHEST COMBO: 77".to_string(), Rect::new(10, 650, 260, 24)),
            ("HITS: 219".to_string(), Rect::new(10, 730, 120, 24)),
            ("MISSES: 170".to_string(), Rect::new(10, 770, 160, 24)),
            ("MEAN M5: 77.42 M5".to_string(), Rect::new(10, 810, 260, 24)),
            ("KPS: 18".to_string(), Rect::new(10, 850, 100, 24)),
            ("SICK: 85".to_string(), Rect::new(2290, 650, 100, 24)),
            ("GOOD: 61".to_string(), Rect::new(2290, 690, 110, 24)),
            ("OK: 47".to_string(), Rect::new(2290, 730, 90, 24)),
            ("BAD: 26".to_string(), Rect::new(2290, 770, 90, 24)),
            ("MISS: 170".to_string(), Rect::new(2290, 810, 110, 24)),
            ("731MS".to_string(), Rect::new(1204, 786, 148, 16)),
            ("-141.65 ms".to_string(), Rect::new(2090, 785, 190, 22)),
        ];
        let f = locate(&lines, Some((870, 1690)), 2560);
        assert!(f.stats.unwrap().x < 20 && f.counts.unwrap().x > 2200);
        assert_eq!(
            f.ms.map(|r| r.x < 1300),
            Some(true),
            "ours, not the opponent's -141.65"
        );
        assert_eq!(f.title.unwrap().y, 4);
        assert_eq!(ms_value("72.71 mS"), Some(72.71));
        assert_eq!(ms_value("-14S.62 ms"), Some(-145.62));
        assert_eq!(ms_value("771 m5"), None, "no dot: not trusted");
        assert_eq!(
            title("ONLINE V'S CHALLENGES - ACCELERANT (INSANE)"),
            ("ONLINE V'S CHALLENGES - ACCELERANT".into(), "INSANE".into())
        );
    }
}
