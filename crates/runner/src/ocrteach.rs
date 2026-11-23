//! OCR as a teacher for the HUD glyph reader (spec 03): OCR text lines in, trusted values out.
//! OCR misreads digits in the game's comic font (`5` as `S`, `1` as `)` or `I`), so a value is
//! only trusted when it parses after repair, at least two OCR passes agree on it, and the
//! panel's arithmetic holds (Hits = Sick + Good + Ok + Bad, Misses = Miss).

use arrowhead_core::frame::Rect;
use arrowhead_core::hud::{Counts, StatsRead};

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
        'G' | 'b' => '6',
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
            '(' | ')' | '|' | '1' | '!' | 'L' if c != 'L' => 'I',
            u => u,
        })
        .collect()
}

/// The text after `label` on the line whose start reads as `label` (not a longer word:
/// MISS is not MISSES), in the line's own characters.
fn after<'a>(lines: &'a Lines, label: &str) -> Option<(&'a str, Rect)> {
    lines.iter().find_map(|(t, r)| {
        if !t.is_ascii() {
            return None;
        }
        let lead = t.len() - t.trim_start().len();
        let n = norm_label(&t[lead..]);
        let next = n[label.len().min(n.len())..].chars().next();
        (n.starts_with(label) && !next.is_some_and(|c| c.is_ascii_alphabetic()))
            .then(|| (&t[lead + label.len()..], *r))
    })
}

/// Value text with digit look-alikes repaired; `None` if anything else is in it.
fn digits(rest: &str) -> Option<String> {
    let v: Option<String> = rest
        .trim_start_matches([':', ';', ' ', '.', '?', '\''])
        .chars()
        .filter(|c| !c.is_whitespace())
        .map(repair)
        .collect();
    v.filter(|v| !v.is_empty())
}

fn uint(lines: &Lines, label: &str) -> Option<u32> {
    digits(after(lines, label)?.0)?.parse().ok()
}

/// `nn.nn` with the dot put back where the format says it goes (OCR drops it).
fn two_dp(digits: &str) -> Option<f32> {
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
        let rs: Vec<Rect> = labels
            .iter()
            .filter_map(|l| after(lines, l).map(|x| x.1))
            .collect();
        // most rows must be seen, else it is some other text
        let row_h = rs.first().map_or(0, |r| r.h);
        (rs.len() * 3 >= labels.len() * 2)
            .then(|| union(&rs).map(|u| padded(u, row_h / 2)))
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
    let ms = lines
        .iter()
        .filter(|(t, r)| {
            let c = r.x + r.w as i32 / 2;
            !norm_label(t).contains("MEAN")
                && ms_value(t).is_some()
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

#[cfg(test)]
mod tests {
    use super::*;

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
        // Misses read 70 / 170 and Miss 70 / 170, once each: 70 = 70 and 170 = 170 tie, so
        // neither is trusted until a pass settles it.
        assert_eq!((s.misses, c.miss), (None, None));
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
            ("72.71 mS".to_string(), Rect::new(1200, 785, 150, 22)),
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
