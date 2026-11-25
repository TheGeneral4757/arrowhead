//! Pure drawing: a `Draw` state into a tiny-skia pixmap. No display needed, so it is unit tested.
//! Layout coordinates are captured-stream pixels; the pixmap is the surface in physical pixels,
//! so everything is scaled by `pixmap width / stream width`. Colours are the app's tokens
//! (`ui/src/tokens.css`) converted to sRGB.

use arrowhead_core::config::Scroll;
use arrowhead_core::session::State;
use tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, Rect, Stroke, Transform};

use crate::model::Draw;

const LANE: [[u8; 3]; 4] = [
    [0xB0, 0x73, 0xDA],
    [0x41, 0xBA, 0xE4],
    [0x68, 0xCB, 0x6E],
    [0xF0, 0x5F, 0x5A],
];
const PAPER: [u8; 3] = [0x0D, 0x10, 0x13];
const INK: [u8; 3] = [0xEC, 0xEF, 0xF2];
const MUTED: [u8; 3] = [0x9D, 0xA2, 0xA8];
const ACCENT: [u8; 3] = [0xF7, 0xB8, 0x3D];
const COOL: [u8; 3] = [0x6E, 0xBC, 0xFC];
const OK: [u8; 3] = [0x61, 0xCB, 0x7C];
const JUDGE_OK: [u8; 3] = [0xD2, 0xBD, 0x70];
const BAD: [u8; 3] = [0xDE, 0x8F, 0x57];
const MISS: [u8; 3] = [0xF6, 0x6D, 0x67];

/// Hit error bar spans +-this many ms.
pub const BAR_MS: f32 = 100.0;

fn col(c: [u8; 3], a: f32) -> Color {
    Color::from_rgba8(c[0], c[1], c[2], (a * 255.0) as u8)
}

fn fill(pm: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, c: Color) {
    if let Some(r) = Rect::from_xywh(x, y, w, h) {
        let mut p = Paint::default();
        p.set_color(c);
        pm.fill_rect(r, &p, Transform::identity(), None);
    }
}

fn judge_colour(j: &str) -> [u8; 3] {
    match j.to_ascii_lowercase().as_str() {
        "sick" => COOL,
        "good" => OK,
        "ok" => JUDGE_OK,
        "bad" => BAD,
        "miss" => MISS,
        _ => INK,
    }
}

/// 5x7 capitals, rows top to bottom, bit 4 is the left column.
#[rustfmt::skip]
const FONT: [[u8; 7]; 26] = [
    [0b01110,0b10001,0b10001,0b11111,0b10001,0b10001,0b10001],
    [0b11110,0b10001,0b10001,0b11110,0b10001,0b10001,0b11110],
    [0b01110,0b10001,0b10000,0b10000,0b10000,0b10001,0b01110],
    [0b11110,0b10001,0b10001,0b10001,0b10001,0b10001,0b11110],
    [0b11111,0b10000,0b10000,0b11110,0b10000,0b10000,0b11111],
    [0b11111,0b10000,0b10000,0b11110,0b10000,0b10000,0b10000],
    [0b01110,0b10001,0b10000,0b10111,0b10001,0b10001,0b01111],
    [0b10001,0b10001,0b10001,0b11111,0b10001,0b10001,0b10001],
    [0b01110,0b00100,0b00100,0b00100,0b00100,0b00100,0b01110],
    [0b00111,0b00010,0b00010,0b00010,0b00010,0b10010,0b01100],
    [0b10001,0b10010,0b10100,0b11000,0b10100,0b10010,0b10001],
    [0b10000,0b10000,0b10000,0b10000,0b10000,0b10000,0b11111],
    [0b10001,0b11011,0b10101,0b10101,0b10001,0b10001,0b10001],
    [0b10001,0b11001,0b10101,0b10011,0b10001,0b10001,0b10001],
    [0b01110,0b10001,0b10001,0b10001,0b10001,0b10001,0b01110],
    [0b11110,0b10001,0b10001,0b11110,0b10000,0b10000,0b10000],
    [0b01110,0b10001,0b10001,0b10001,0b10101,0b10010,0b01101],
    [0b11110,0b10001,0b10001,0b11110,0b10100,0b10010,0b10001],
    [0b01111,0b10000,0b10000,0b01110,0b00001,0b00001,0b11110],
    [0b11111,0b00100,0b00100,0b00100,0b00100,0b00100,0b00100],
    [0b10001,0b10001,0b10001,0b10001,0b10001,0b10001,0b01110],
    [0b10001,0b10001,0b10001,0b10001,0b10001,0b01010,0b00100],
    [0b10001,0b10001,0b10001,0b10101,0b10101,0b10101,0b01010],
    [0b10001,0b10001,0b01010,0b00100,0b01010,0b10001,0b10001],
    [0b10001,0b10001,0b01010,0b00100,0b00100,0b00100,0b00100],
    [0b11111,0b00001,0b00010,0b00100,0b01000,0b10000,0b11111],
];

/// Width of `s` in pixels at pixel size `px` (6 columns per glyph, the last is spacing).
fn text_w(s: &str, px: f32) -> f32 {
    (s.chars().count() as f32 * 6.0 - 1.0) * px
}

#[rustfmt::skip]
const DIGITS: [[u8; 7]; 10] = [
    [0b01110,0b10001,0b10011,0b10101,0b11001,0b10001,0b01110],
    [0b00100,0b01100,0b00100,0b00100,0b00100,0b00100,0b01110],
    [0b01110,0b10001,0b00001,0b00010,0b00100,0b01000,0b11111],
    [0b11110,0b00001,0b00001,0b01110,0b00001,0b00001,0b11110],
    [0b00010,0b00110,0b01010,0b10010,0b11111,0b00010,0b00010],
    [0b11111,0b10000,0b11110,0b00001,0b00001,0b10001,0b01110],
    [0b00110,0b01000,0b10000,0b11110,0b10001,0b10001,0b01110],
    [0b11111,0b00001,0b00010,0b00100,0b01000,0b01000,0b01000],
    [0b01110,0b10001,0b10001,0b01110,0b10001,0b10001,0b01110],
    [0b01110,0b10001,0b10001,0b01111,0b00001,0b00010,0b01100],
];
const MINUS: [u8; 7] = [0, 0, 0, 0b11111, 0, 0, 0];
const DOT: [u8; 7] = [0, 0, 0, 0, 0, 0b01100, 0b01100];
const QUESTION: [u8; 7] = [0b01110, 0b10001, 0b00001, 0b00110, 0b00100, 0, 0b00100];

/// Capitals, digits, `-`, `.`, `?` and spaces; anything else is skipped.
fn text(pm: &mut Pixmap, s: &str, x: f32, y: f32, px: f32, c: Color) {
    for (i, ch) in s.chars().enumerate() {
        let ch = ch.to_ascii_uppercase();
        let glyph = match ch {
            'A'..='Z' => &FONT[(ch as u8 - b'A') as usize],
            '0'..='9' => &DIGITS[(ch as u8 - b'0') as usize],
            '-' => &MINUS,
            '.' => &DOT,
            '?' => &QUESTION,
            _ => continue,
        };
        for (row, bits) in glyph.iter().enumerate() {
            for colm in 0..5 {
                if bits >> (4 - colm) & 1 == 1 {
                    let gx = x + (i as f32 * 6.0 + colm as f32) * px;
                    fill(pm, gx, y + row as f32 * px, px, px, c);
                }
            }
        }
    }
}

pub fn state_label(state: State, dry_run: bool) -> (&'static str, [u8; 3]) {
    match (state, dry_run) {
        (State::Playing, true) => ("DRY RUN", ACCENT),
        (State::Playing, false) => ("PLAYING", OK),
        (State::Armed, _) => ("ARMED", COOL),
        (State::Searching, _) => ("SEARCHING", JUDGE_OK),
        (State::Paused, _) => ("PAUSED", JUDGE_OK),
        (State::Results, _) => ("RESULTS", MUTED),
        (State::Idle, _) => ("IDLE", MUTED),
    }
}

/// Panel size in UI units: state pill and judgment on one row, the hit error bar below.
const PANEL_W: f32 = 440.0;
const PANEL_H: f32 = 72.0;

fn overlaps(a: [f32; 4], b: [f32; 4]) -> f32 {
    let w = (a[0] + a[2]).min(b[0] + b[2]) - a[0].max(b[0]);
    let h = (a[1] + a[3]).min(b[1] + b[3]) - a[1].max(b[1]);
    w.max(0.0) * h.max(0.0)
}

/// Where the panel goes (surface px, top-left): the first of top-left, top-right, top-centre,
/// bottom-centre that covers no HUD region and not our playfield, else the one that covers
/// least. The game is read from what is under it (spec 13, 2.1).
fn panel_at(d: &Draw, pw: f32, ph: f32, u: f32) -> (f32, f32) {
    let (w, h, m) = (PANEL_W * u, PANEL_H * u, 16.0 * u);
    let mut keep_out: Vec<[f32; 4]> = vec![];
    if let Some(l) = d.layout.as_ref().filter(|l| l.stream[0] > 0) {
        let s = pw / l.stream[0] as f32;
        for (r, _) in &d.hud {
            keep_out.push(r.map(|v| v as f32 * s));
        }
        let xs = l.receptors.map(|r| r[0] * s);
        let lo = xs.iter().copied().fold(f32::MAX, f32::min) - l.size * s;
        let hi = xs.iter().copied().fold(f32::MIN, f32::max) + l.size * s;
        keep_out.push([lo, 0.0, hi - lo, ph]);
    }
    let spots = [
        (m, m),
        (pw - w - m, m),
        ((pw - w) / 2.0, m),
        ((pw - w) / 2.0, ph - h - m),
    ];
    let cost =
        |&(x, y): &(f32, f32)| -> f32 { keep_out.iter().map(|k| overlaps([x, y, w, h], *k)).sum() };
    spots
        .iter()
        .find(|p| cost(p) == 0.0)
        .or_else(|| spots.iter().min_by(|a, b| cost(a).total_cmp(&cost(b))))
        .copied()
        .unwrap_or((m, m))
}

/// Clears `pm` and draws `d`. `pm` is the whole surface in physical pixels.
pub fn render(d: &Draw, pm: &mut Pixmap) {
    pm.fill(Color::TRANSPARENT);
    let pw = pm.width() as f32;
    let ph = pm.height() as f32;
    // UI sizing follows the surface, not the stream.
    let u = (pw / 1920.0).max(0.75);

    if d.debug {
        debug_marks(d, pm, u);
    }

    let (x0, y0) = panel_at(d, pw, ph, u);
    let pill_h = 26.0 * u;
    let mut jx = x0;
    if let Some((state, dry)) = d.state {
        let (label, c) = state_label(state, dry);
        let px = 2.0 * u;
        let w = text_w(label, px) + 24.0 * u;
        fill(pm, x0, y0, w, pill_h, col(PAPER, 0.8));
        fill(pm, x0, y0, 4.0 * u, pill_h, col(c, 1.0));
        text(pm, label, x0 + 14.0 * u, y0 + 6.0 * u, px, col(c, 1.0));
        jx = x0 + w + 12.0 * u;
    }

    if let Some(j) = &d.judgment {
        let px = 3.0 * u;
        let w = text_w(j, px) + 16.0 * u;
        fill(pm, jx, y0, w, pill_h, col(PAPER, 0.8));
        text(
            pm,
            j,
            jx + 8.0 * u,
            y0 + (pill_h - 7.0 * px) / 2.0,
            px,
            col(judge_colour(j), 1.0),
        );
    }

    if !d.errors.is_empty() {
        let (w, h) = (PANEL_W * u, 22.0 * u);
        let y = y0 + pill_h + 16.0 * u;
        let mid = x0 + w / 2.0;
        fill(pm, x0, y + h / 2.0 - 3.0 * u, w, 6.0 * u, col(PAPER, 0.7));
        fill(
            pm,
            mid - u,
            y - 4.0 * u,
            2.0 * u,
            h + 8.0 * u,
            col(INK, 0.9),
        );
        let n = d.errors.len();
        for (i, e) in d.errors.iter().enumerate() {
            let f = (e / BAR_MS).clamp(-1.0, 1.0);
            let a = 0.25 + 0.75 * (i + 1) as f32 / n as f32;
            fill(pm, mid + f * w / 2.0 - 1.5 * u, y, 3.0 * u, h, col(COOL, a));
        }
    }
}

/// `--debug` only, never during a real run: lane strips, receptor marks, planned ticks and
/// the HUD regions with labels. They cover what the engine reads.
fn debug_marks(d: &Draw, pm: &mut Pixmap, u: f32) {
    let Some(l) = &d.layout else { return };
    if l.stream[0] == 0 {
        return;
    }
    let pw = pm.width() as f32;
    let s = pw / l.stream[0] as f32;
    let ph = pm.height() as f32;
    let (size, up) = (l.size * s, l.scroll == Scroll::Up);
    let stroke = Stroke {
        width: (2.0 * u).max(1.0),
        ..Stroke::default()
    };

    for (r, label) in &d.hud {
        let (x, y, w, h) = (
            r[0] as f32 * s,
            r[1] as f32 * s,
            r[2] as f32 * s,
            r[3] as f32 * s,
        );
        let mut p = Paint::default();
        p.set_color(col(ACCENT, 0.8));
        if let Some(rect) = Rect::from_xywh(x, y, w.max(1.0), h.max(1.0)) {
            pm.stroke_path(
                &PathBuilder::from_rect(rect),
                &p,
                &stroke,
                Transform::identity(),
                None,
            );
        }
        let px = 1.5 * u;
        let ly = (y - 7.0 * px - 6.0 * u).max(0.0);
        fill(
            pm,
            x,
            ly,
            text_w(label, px) + 8.0 * u,
            7.0 * px + 4.0 * u,
            col(PAPER, 0.8),
        );
        text(pm, label, x + 4.0 * u, ly + 2.0 * u, px, col(ACCENT, 1.0));
    }

    for (i, r) in l.receptors.iter().enumerate() {
        let (cx, cy) = (r[0] * s, r[1] * s);
        // Strip: from the receptor to the edge notes come from.
        let (y0, y1) = if up { (cy, ph) } else { (0.0, cy) };
        fill(pm, cx - size / 2.0, y0, size, y1 - y0, col(LANE[i], 0.10));
        let mut p = Paint::default();
        p.set_color(col(LANE[i], 0.55));
        if let Some(r) = Rect::from_xywh(cx - size / 2.0, y0, size, y1 - y0) {
            pm.stroke_path(
                &PathBuilder::from_rect(r),
                &p,
                &stroke,
                Transform::identity(),
                None,
            );
        }
        let mut pb = PathBuilder::new();
        pb.push_circle(cx, cy, size / 2.0);
        if let Some(path) = pb.finish() {
            p.set_color(col(LANE[i], 0.25));
            pm.fill_path(&path, &p, FillRule::Winding, Transform::identity(), None);
            p.set_color(col(LANE[i], 1.0));
            pm.stroke_path(&path, &p, &stroke, Transform::identity(), None);
        }
    }

    for &(_, lane, _) in &d.planned {
        if let Some(r) = l.receptors.get(lane as usize) {
            let (cx, cy) = (r[0] * s, r[1] * s);
            let h = 5.0 * u;
            let y = if up {
                cy - size * 0.8 - h
            } else {
                cy + size * 0.8
            };
            fill(pm, cx - size * 0.4, y, size * 0.8, h, col(INK, 0.9));
        }
    }
}

/// tiny-skia is premultiplied RGBA; Wayland ARGB8888 and Win32 DIBs are premultiplied BGRA.
pub fn to_bgra(pm: &Pixmap, out: &mut [u8]) {
    for (o, p) in out.chunks_exact_mut(4).zip(pm.data().chunks_exact(4)) {
        o.copy_from_slice(&[p[2], p[1], p[0], p[3]]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Layout, Msg};

    fn drawn() -> Draw {
        Draw {
            layout: Some(Layout {
                receptors: [
                    [400.0, 200.0],
                    [500.0, 200.0],
                    [600.0, 200.0],
                    [700.0, 200.0],
                ],
                size: 60.0,
                scroll: Scroll::Up,
                stream: [1000, 600],
            }),
            ..Draw::default()
        }
    }

    fn px(pm: &Pixmap, x: u32, y: u32) -> [u8; 4] {
        let p = pm.pixel(x, y).unwrap();
        [p.red(), p.green(), p.blue(), p.alpha()]
    }

    fn debug(mut d: Draw) -> Draw {
        d.debug = true;
        d
    }

    /// Every pixel in our playfield column (receptors +- size, at 2x) is untouched.
    fn playfield_clear(pm: &Pixmap) -> bool {
        (0..pm.height()).all(|y| (680..=1520).all(|x| px(pm, x, y)[3] == 0))
    }

    #[test]
    fn run_draws_nothing_over_the_playfield_or_hud() {
        let mut d = drawn();
        d.hud = vec![([0, 0, 200, 60], "stats".into())];
        d.apply(Msg::State {
            state: State::Playing,
            dry_run: false,
        });
        d.apply(Msg::Planned {
            id: 7,
            lane: 0,
            at_ms: 10.0,
        });
        d.apply(Msg::Readback {
            ms: 50.0,
            judgment: "sick".into(),
        });
        let mut pm = Pixmap::new(2000, 1200).unwrap();
        render(&d, &mut pm);
        assert!(playfield_clear(&pm));
        // the stats region in the top-left is avoided too
        assert!((0..120).all(|y| (0..400).all(|x| px(&pm, x, y)[3] == 0)));
        // the pill went top-right, the free corner
        assert!((1520..2000).any(|x| px(&pm, x, 25)[3] > 200));
    }

    #[test]
    fn debug_strips_and_receptors_at_stream_scale() {
        let d = debug(drawn());
        let mut pm = Pixmap::new(1000, 600).unwrap();
        render(&d, &mut pm);
        let p = px(&pm, 400, 400);
        assert!(p[3] > 10 && p[3] < 60, "{p:?}");
        assert_eq!(px(&pm, 450, 400)[3], 0);
        assert!(px(&pm, 470, 200)[3] > 200);
        let mut pm = Pixmap::new(2000, 1200).unwrap();
        render(&d, &mut pm);
        assert!(px(&pm, 800, 800)[3] > 10);
        assert_eq!(px(&pm, 900, 800)[3], 0);
    }

    #[test]
    fn debug_planned_tick_until_pressed() {
        let mut d = debug(drawn());
        let mut pm = Pixmap::new(1000, 600).unwrap();
        d.apply(Msg::Planned {
            id: 7,
            lane: 0,
            at_ms: 10.0,
        });
        render(&d, &mut pm);
        assert!(px(&pm, 400, 200 - 48 - 2)[3] > 200);
        d.apply(Msg::Pressed { id: 7 });
        render(&d, &mut pm);
        assert_eq!(px(&pm, 400, 200 - 48 - 2)[3], 0);
    }

    #[test]
    fn pill_judgment_and_error_bar_in_the_panel() {
        let mut d = Draw::default();
        d.apply(Msg::State {
            state: State::Playing,
            dry_run: false,
        });
        d.apply(Msg::Readback {
            ms: 50.0,
            judgment: "sick".into(),
        });
        let mut pm = Pixmap::new(1000, 600).unwrap();
        render(&d, &mut pm);
        // u = 0.75; nothing known, so top-left at 12, 12
        assert_eq!(&px(&pm, 13, 20)[..3], &[0x61, 0xCB, 0x7C]);
        // bar: y = 12 + 19.5 + 12 = 43.5, 16.5 high; +50 ms is a quarter right of the middle
        let (w, mid) = (440.0 * 0.75, 12.0 + 440.0 * 0.75 / 2.0);
        let x = (mid + 0.5 * w / 2.0) as u32;
        assert!(px(&pm, x, 50)[3] > 200);
    }

    #[test]
    fn bgra_swaps_red_and_blue() {
        let mut pm = Pixmap::new(1, 1).unwrap();
        pm.fill(Color::from_rgba8(1, 2, 3, 255));
        let mut o = [0u8; 4];
        to_bgra(&pm, &mut o);
        assert_eq!(o, [3, 2, 1, 255]);
    }
}
