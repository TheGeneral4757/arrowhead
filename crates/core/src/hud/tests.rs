//! Synthetic HUD frames: 5x7 bitmap digits (and hashed patterns for every other
//! character) drawn in per-element colours on black. No game images.

use super::*;
use crate::time::Nanos;

const DIGITS: [[&str; 7]; 10] = [
    [
        "01110", "10001", "10011", "10101", "11001", "10001", "01110",
    ],
    [
        "00100", "01100", "00100", "00100", "00100", "00100", "01110",
    ],
    [
        "01110", "10001", "00001", "00010", "00100", "01000", "11111",
    ],
    [
        "11110", "00001", "00001", "01110", "00001", "00001", "11110",
    ],
    [
        "00010", "00110", "01010", "10010", "11111", "00010", "00010",
    ],
    [
        "11111", "10000", "11110", "00001", "00001", "10001", "01110",
    ],
    [
        "00110", "01000", "10000", "11110", "10001", "10001", "01110",
    ],
    [
        "11111", "00001", "00010", "00100", "01000", "01000", "01000",
    ],
    [
        "01110", "10001", "10001", "01110", "10001", "10001", "01110",
    ],
    [
        "01110", "10001", "10001", "01111", "00001", "00010", "01100",
    ],
];

/// Rows of '0'/'1' for a character; letters and symbols get a stable pseudo-random 5x7.
fn glyph(c: char) -> Vec<String> {
    match c {
        '0'..='9' => DIGITS[c as usize - '0' as usize]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        '.' => (0..7)
            .map(|y| if y == 6 { "1" } else { "0" }.to_string())
            .collect(),
        '-' => (0..7)
            .map(|y| if y == 3 { "111" } else { "000" }.to_string())
            .collect(),
        _ => {
            let mut h = (c as u32).wrapping_mul(2654435761) | 1;
            (0..7)
                .map(|y| {
                    (0..5)
                        .map(|x| {
                            h = h.wrapping_mul(1664525).wrapping_add(1013904223);
                            // a spine keeps the glyph one connected column run
                            if x == 2 || y == 3 || (h >> 24) & 3 == 0 {
                                '1'
                            } else {
                                '0'
                            }
                        })
                        .collect()
                })
                .collect()
        }
    }
}

const S: usize = 4;

struct Canvas {
    w: usize,
    h: usize,
    data: Vec<u8>,
}

impl Canvas {
    fn new(w: usize, h: usize) -> Self {
        Self {
            w,
            h,
            data: vec![0; w * h * 4],
        }
    }

    /// Draws `text` (spaces are a 3-module gap) with its top-left at (x, y); returns end x.
    fn text(&mut self, mut x: usize, y: usize, text: &str, rgb: (u8, u8, u8)) -> usize {
        for c in text.chars() {
            if c == ' ' {
                x += 4 * S;
                continue;
            }
            let g = glyph(c);
            for (gy, row) in g.iter().enumerate() {
                for (gx, b) in row.bytes().enumerate() {
                    if b == b'1' {
                        for dy in 0..S {
                            for dx in 0..S {
                                let i = ((y + gy * S + dy) * self.w + x + gx * S + dx) * 4;
                                self.data[i..i + 3].copy_from_slice(&[rgb.2, rgb.1, rgb.0]);
                            }
                        }
                    }
                }
            }
            x += (g[0].len() + 1) * S;
        }
        x
    }

    fn frame(&self) -> OwnedFrame {
        OwnedFrame {
            seq: 0,
            pts: Nanos(0),
            region: Rect::new(0, 0, self.w as u32, self.h as u32),
            data: self.data.clone(),
        }
    }
}

use crate::frame::OwnedFrame;

const COLOURS: [(u8, u8, u8); 7] = [
    (255, 255, 255),
    (255, 80, 255),
    (60, 255, 60),
    (255, 255, 80),
    (250, 90, 90),
    (255, 255, 255),
    (200, 200, 200),
];

fn stack(c: &mut Canvas, x: usize, y: usize, l: Layout, values: &[&str]) {
    for (i, (lab, v)) in l.labels().iter().zip(values).enumerate() {
        let yy = y + i * 44;
        let end = c.text(x, yy, &format!("{lab}:"), COLOURS[i % 7]);
        c.text(end + 4 * S, yy, v, COLOURS[(i + 3) % 7]);
    }
}

/// Our playfield in the synthetic scene: our ms readout sits inside it, the opponent's not.
const OURS: (i32, i32) = (400, 860);

fn scene(stats: &[&str], counts: &[&str], ms: Option<&str>, scores: [&str; 2]) -> OwnedFrame {
    scene_with(stats, counts, ms, None, scores)
}

fn scene_with(
    stats: &[&str],
    counts: &[&str],
    ms: Option<&str>,
    opponent_ms: Option<&str>,
    scores: [&str; 2],
) -> OwnedFrame {
    let mut c = Canvas::new(1280, 720);
    stack(&mut c, 20, 200, Layout::Stats, stats);
    stack(&mut c, 900, 200, Layout::Counts, counts);
    if let Some(ms) = ms {
        c.text(560, 400, ms, (255, 255, 80));
    }
    if let Some(ms) = opponent_ms {
        c.text(900, 500, ms, (255, 255, 80));
    }
    c.text(30, 640, scores[0], (255, 255, 255));
    c.text(1000, 640, scores[1], (255, 255, 255));
    c.frame()
}

const TEACH_STATS: [[&str; 7]; 2] = [
    ["12.34%", "5678", "90", "1234", "5678", "90.12ms", "345"],
    ["56.78%", "9012", "34", "5678", "9012", "34.56ms", "789"],
];
const TEACH_COUNTS: [[&str; 5]; 2] = [
    ["12345", "67890", "1", "22", "333"],
    ["1", "2", "3", "4", "5"],
];

fn taught() -> HudReader {
    let mut rd = HudReader::new(Atlas::default(), 0.25);
    rd.playfield = Some(OURS);
    // two teaching frames cover every digit in every face
    let a = scene(
        &TEACH_STATS[0],
        &TEACH_COUNTS[0],
        Some("-1234.56ms"),
        ["-1234567", "8909"],
    );
    let b = scene(
        &TEACH_STATS[1],
        &TEACH_COUNTS[1],
        Some("-90.78ms"),
        ["-9", "80"],
    );
    for (f, i) in [(&a, 0), (&b, 1)] {
        let v = f.view();
        rd.teach_stack(&v, Layout::Stats, &TEACH_STATS[i]).unwrap();
        rd.teach_stack(&v, Layout::Counts, &TEACH_COUNTS[i])
            .unwrap();
    }
    let ours = Some(Rect::new(OURS.0, 0, (OURS.1 - OURS.0) as u32, 720));
    rd.teach_lines(&a.view(), Face::Ms, &["-1234.56ms"], ours)
        .unwrap();
    rd.teach_lines(&b.view(), Face::Ms, &["-90.78ms"], ours)
        .unwrap();
    rd.teach_lines(&a.view(), Face::Score, &["-1234567", "8909"], None)
        .unwrap();
    rd.teach_lines(&b.view(), Face::Score, &["-9", "80"], None)
        .unwrap();
    rd
}

#[test]
fn parsers() {
    assert_eq!(parse_ms("-143.26ms"), Some(-143.26));
    assert_eq!(parse_ms("12.50ms"), Some(12.5));
    assert_eq!(parse_ms("143.26"), None);
    assert_eq!(parse_ms("-.26ms"), None);
    assert_eq!(parse_ms("1.2ms"), None);
    assert_eq!(parse_pct("84.48%"), Some(84.48));
    assert_eq!(parse_pct("104.00%"), None);
    assert_eq!(parse_int("-395"), Some(-395));
    assert_eq!(parse_int("39-5"), None);
    assert_eq!(parse_uint(""), None);
    assert_eq!(parse_uint("12345678"), None);
    assert_eq!(Judgment::parse("Sick!"), Some(Judgment::Sick));
    assert_eq!(Judgment::parse("Miss"), Some(Judgment::Miss));
    assert_eq!(Judgment::parse("Great"), None);
}

#[test]
fn locate_and_read_synthetic_hud() {
    let mut rd = taught();
    // a different frame: new numbers, all from taught digits; the opponent's readout is
    // further right, outside our playfield, and must not be taken for ours
    let f = scene_with(
        &["98.76%", "4321", "0987", "5612", "3490", "87.65ms", "21"],
        &["10", "20", "30", "40", "5"],
        Some("-43.21ms"),
        Some("-12.34ms"),
        ["-9876", "5432"],
    );
    let v = f.view();
    let loc = rd.locate(&v);
    assert!(loc.stats.is_some() && loc.counts.is_some() && loc.ms.is_some());
    assert!(
        loc.score_left.is_some() && loc.score_right.is_some(),
        "{loc:?}"
    );
    let r = rd.read(&v);
    assert_eq!(
        r.stats,
        Some(StatsRead {
            accuracy: Some(98.76),
            highest_combo: Some(4321),
            combo: Some(987),
            hits: Some(5612),
            misses: Some(3490),
            mean_ms: Some(87.65),
            kps: Some(21)
        })
    );
    assert_eq!(
        r.counts,
        Some(Counts {
            sick: Some(10),
            good: Some(20),
            ok: Some(30),
            bad: Some(40),
            miss: Some(5)
        })
    );
    assert_eq!(r.ms, Some(-43.21));
    assert_eq!(r.scores, [Some(-9876), Some(5432)]);

    // no ms text: blank, not a misread
    let f2 = scene(
        &["98.76%", "4321", "0987", "5612", "3490", "87.65ms", "21"],
        &["10", "20", "30", "40", "5"],
        None,
        ["-9876", "5432"],
    );
    let r2 = rd.read(&f2.view());
    assert!(r2.ms.is_none() && r2.ms_blank);

    // only the opponent's readout on screen (we have not hit yet): no ms region at all
    let mut rd = taught();
    let f3 = scene_with(
        &["98.76%", "4321", "0987", "5612", "3490", "87.65ms", "21"],
        &["10", "20", "30", "40", "5"],
        None,
        Some("-12.34ms"),
        ["-9876", "5432"],
    );
    assert_eq!(rd.locate(&f3.view()).ms, None);
}

#[test]
fn ms_font_taught_from_the_first_hit() {
    // Stats taught, ms font not: the frame where Hits first reads 1 teaches it from Mean MS.
    let mut rd = HudReader::new(Atlas::default(), 0.25);
    rd.playfield = Some(OURS);
    let a = scene(&TEACH_STATS[0], &TEACH_COUNTS[0], None, ["0", "0"]);
    rd.teach_stack(&a.view(), Layout::Stats, &TEACH_STATS[0])
        .unwrap();
    let first = scene_with(
        &["0.00%", "1", "1", "1", "0", "-12.34ms", "1"],
        &["1", "0", "0", "0", "0"],
        Some("-12.34ms"),
        Some("-56.78ms"),
        ["350", "0"],
    );
    assert!(rd.teach_ms_first_hit(&first.view(), -12.34));
    assert_eq!(rd.read(&first.view()).ms, Some(-12.34));
    // a readout made of known glyphs that says something else than Mean MS: nothing is kept
    let mut rd2 = HudReader::new(rd.atlas.clone(), 0.25);
    rd2.playfield = Some(OURS);
    let before = rd2.atlas.to_bytes();
    let wrong = scene(
        &TEACH_STATS[1],
        &TEACH_COUNTS[1],
        Some("-21.43ms"),
        ["0", "0"],
    );
    assert!(!rd2.teach_ms_first_hit(&wrong.view(), -12.34));
    assert_eq!(rd2.atlas.to_bytes(), before);
}

#[test]
fn unknown_glyphs_are_rejected_not_guessed() {
    let mut rd = taught();
    // 'x' was never taught as a digit; a stats row containing it must not parse
    let f = scene(
        &["98.76%", "4x21", "0987", "5612", "3490", "87.65ms", "21"],
        &["10", "20", "30", "40", "5"],
        Some("-43.21ms"),
        ["-9876", "5432"],
    );
    let v = f.view();
    rd.locate(&v);
    let r = rd.read(&v);
    let s = r.stats.unwrap();
    assert_eq!(s.highest_combo, None);
    assert_eq!(s.combo, Some(987));
}

#[test]
fn atlas_bytes_roundtrip() {
    let rd = taught();
    let b = rd.atlas.to_bytes();
    let back = Atlas::from_bytes(&b).unwrap();
    assert_eq!(back.to_bytes(), b);
    assert!(Atlas::from_bytes(&b[..b.len() - 3]).is_none());
    assert!(Atlas::from_bytes(b"nope").is_none());

    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("glyphs").join("default.bin");
    rd.atlas.save(&p).unwrap();
    assert_eq!(Atlas::load(&p).to_bytes(), b);
    assert!(Atlas::load(&dir.path().join("missing.bin")).is_empty());
}

#[test]
fn regions_roundtrip_through_profile() {
    let mut rd = taught();
    let f = scene(
        &["98.76%", "4321", "0987", "5612", "3490", "87.65ms", "21"],
        &["10", "20", "30", "40", "5"],
        Some("-43.21ms"),
        ["-9876", "5432"],
    );
    let loc = rd.locate(&f.view());
    let back = Locations::from_regions(&loc.to_regions());
    assert_eq!(
        (back.stats, back.counts, back.ms),
        (loc.stats, loc.counts, loc.ms)
    );
}

#[test]
fn teach_row_learns_a_digit_and_refuses_a_wrong_one() {
    // Bootstrapped from zeros only, then Hits ticks 0 -> 1.
    let mut rd = HudReader::new(Atlas::default(), 0.25);
    let zero = scene(
        &["0.00%", "0", "0", "0", "0", "0.00ms", "0"],
        &["0"; 5],
        None,
        ["0", "0"],
    );
    rd.teach_stack(
        &zero.view(),
        Layout::Stats,
        &["0.00%", "0", "0", "0", "0", "0.00ms", "0"],
    )
    .unwrap();
    let one = scene(
        &["0.00%", "1", "1", "1", "0", "0.00ms", "0"],
        &["0"; 5],
        None,
        ["0", "0"],
    );
    let stats = rd.locate(&one.view()).stats.unwrap();
    assert_eq!(
        rd.read_stats(&one.view(), stats).hits,
        None,
        "1 not taught yet"
    );
    assert!(rd.teach_row(&one.view(), Layout::Stats, stats, 3, "1"));
    assert_eq!(rd.read_stats(&one.view(), stats).hits, Some(1));
    // Combo shows 1 too; claiming it shows 0 contradicts the known '0' and is refused.
    let before = rd.atlas.to_bytes();
    assert!(!rd.teach_row(&one.view(), Layout::Stats, stats, 2, "0"));
    assert_eq!(rd.atlas.to_bytes(), before);
}
