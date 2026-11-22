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

/// Capitals and spaces only; anything else is skipped.
fn text(pm: &mut Pixmap, s: &str, x: f32, y: f32, px: f32, c: Color) {
    for (i, ch) in s.chars().enumerate() {
        let ch = ch.to_ascii_uppercase();
        if !ch.is_ascii_uppercase() {
            continue;
        }
        for (row, bits) in FONT[(ch as u8 - b'A') as usize].iter().enumerate() {
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

/// Clears `pm` and draws `d`. `pm` is the whole surface in physical pixels.
pub fn render(d: &Draw, pm: &mut Pixmap) {
    pm.fill(Color::TRANSPARENT);
    let pw = pm.width() as f32;
    // UI sizing follows the surface, not the stream.
    let u = (pw / 1920.0).max(0.75);

    if let Some((state, dry)) = d.state {
        let (label, c) = state_label(state, dry);
        let px = 2.0 * u;
        let (w, h) = (text_w(label, px) + 24.0 * u, 7.0 * px + 12.0 * u);
        let (x, y) = (16.0 * u, 16.0 * u);
        fill(pm, x, y, w, h, col(PAPER, 0.8));
        fill(pm, x, y, 4.0 * u, h, col(c, 1.0));
        text(pm, label, x + 14.0 * u, y + 6.0 * u, px, col(c, 1.0));
    }

    let Some(l) = &d.layout else { return };
    if l.stream[0] == 0 {
        return;
    }
    let s = pw / l.stream[0] as f32;
    let ph = pm.height() as f32;
    let (size, up) = (l.size * s, l.scroll == Scroll::Up);
    let stroke = Stroke {
        width: (2.0 * u).max(1.0),
        ..Stroke::default()
    };

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
        // Receptor mark.
        let mut pb = PathBuilder::new();
        pb.push_circle(cx, cy, size / 2.0);
        if let Some(path) = pb.finish() {
            p.set_color(col(LANE[i], 0.25));
            pm.fill_path(&path, &p, FillRule::Winding, Transform::identity(), None);
            p.set_color(col(LANE[i], 1.0));
            pm.stroke_path(&path, &p, &stroke, Transform::identity(), None);
        }
    }

    // Planned press ticks: a bar on the far side of the receptor.
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

    let mid = l.receptors.iter().map(|r| r[0]).sum::<f32>() / 4.0 * s;
    let ry = l.receptors[0][1] * s;

    if let Some(j) = &d.judgment {
        let px = 4.0 * u;
        let y = if up { ry - size * 2.2 } else { ry + size * 2.2 } - 3.5 * px;
        text(
            pm,
            j,
            mid - text_w(j, px) / 2.0,
            y,
            px,
            col(judge_colour(j), 1.0),
        );
    }

    if !d.errors.is_empty() {
        let (w, h) = (420.0 * u, 22.0 * u);
        let (x, y) = (mid - w / 2.0, ph - 70.0 * u);
        fill(pm, x, y + h / 2.0 - 3.0 * u, w, 6.0 * u, col(PAPER, 0.7));
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

    #[test]
    fn strips_and_receptors_at_stream_scale() {
        let d = drawn();
        let mut pm = Pixmap::new(1000, 600).unwrap();
        render(&d, &mut pm);
        // Inside lane 1's strip, below the receptor: faint lane colour.
        let p = px(&pm, 400, 400);
        assert!(p[3] > 10 && p[3] < 60, "{p:?}");
        // Between strips and above the receptors (scroll up): untouched.
        assert_eq!(px(&pm, 450, 400)[3], 0);
        assert_eq!(px(&pm, 400, 100)[3], 0);
        // Receptor ring on the left edge of lane 2.
        assert!(px(&pm, 470, 200)[3] > 200);
    }

    #[test]
    fn scales_to_a_bigger_surface() {
        let d = drawn();
        let mut pm = Pixmap::new(2000, 1200).unwrap();
        render(&d, &mut pm);
        assert!(px(&pm, 800, 800)[3] > 10);
        assert_eq!(px(&pm, 900, 800)[3], 0);
    }

    #[test]
    fn planned_tick_until_pressed() {
        let mut d = drawn();
        let mut pm = Pixmap::new(1000, 600).unwrap();
        d.apply(Msg::Planned {
            id: 7,
            lane: 0,
            at_ms: 10.0,
        });
        render(&d, &mut pm);
        // 0.8 * size above the receptor, scroll up.
        assert!(px(&pm, 400, 200 - 48 - 2)[3] > 200);
        d.apply(Msg::Pressed { id: 7 });
        render(&d, &mut pm);
        assert_eq!(px(&pm, 400, 200 - 48 - 2)[3], 0);
    }

    #[test]
    fn pill_judgment_and_error_bar() {
        let mut d = drawn();
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
        // u = 0.75 here (surface narrower than 1920). Pill edge at x = 12, y = 12.
        assert_eq!(&px(&pm, 13, 20)[..3], &[0x61, 0xCB, 0x7C]);
        // +50 ms is a quarter bar right of the centre line (mid 550, bar 315 wide).
        let x = (550.0 + 0.5 * 315.0 / 2.0) as u32;
        assert!(px(&pm, x, 600 - 53 + 8)[3] > 200);
        // Judgment text: an opaque pixel somewhere in its row band above the receptors.
        let y = (200.0 - 60.0 * 2.2 - 3.5 * 3.0) as u32 + 8;
        assert!((0..1000).any(|x| px(&pm, x, y)[3] > 200));
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
