//! OCR teacher for the HUD reader: wraps the `ocrs` engine (pure Rust, offline).

use std::error::Error;
use std::path::{Path, PathBuf};

use arrowhead_core::frame::{FrameView, Rect};
use ocrs::{ImageSource, OcrEngine, OcrEngineParams, TextItem};
use rten::Model;

mod paddle;

const DET: &str = "text-detection.onnx";
const REC: &str = "text-recognition.onnx";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variant {
    Plain2x,
    Lanczos3x,
    Threshold2x,
}

enum Engine {
    Ocrs(OcrEngine),
    /// Larger PP-OCR set (a dir with `dict.txt`), see `paddle.rs`.
    Paddle(Box<paddle::Paddle>),
}

pub struct Ocr {
    engine: Engine,
}

/// Dirs searched for model set `set` ("default" = the models dir itself, else `models/<set>/`).
fn search_dirs(set: &str) -> Vec<PathBuf> {
    let sub = |p: PathBuf| if set == "default" { p } else { p.join(set) };
    let mut v = Vec::new();
    if let Some(d) = std::env::var_os("ARROWHEAD_MODELS") {
        v.push(sub(d.into()));
    }
    if let Some(d) = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|p| p.join("models")))
    {
        v.push(sub(d));
    }
    v.push(sub("/usr/share/arrowhead/models".into()));
    v.push(sub("/usr/lib/arrowhead/models".into()));
    if let Some(h) = dirs::home_dir() {
        v.push(sub(h.join(".cache/ocrs"))); // dev only: ocrs-cli downloads here
    }
    v
}

/// OCR threads at most (rten's default pool takes every core, which would compete with the
/// game and the scan thread while a teaching pass runs).
pub const THREADS: &str = "2";

/// Caps rten's thread pool. Call first thing in `main`, before any thread exists: it sets an
/// environment variable, which is only sound while the process is single-threaded.
pub fn limit_threads() {
    if std::env::var_os("RTEN_NUM_THREADS").is_none() {
        // SAFETY: called at the top of main, before any other thread is started.
        unsafe { std::env::set_var("RTEN_NUM_THREADS", THREADS) };
    }
}

pub fn find_models(set: &str) -> Option<PathBuf> {
    search_dirs(set)
        .into_iter()
        .find(|d| d.join(DET).is_file() && d.join(REC).is_file())
}

impl Ocr {
    /// Loads model set `model` ("default" = the bundled ocrs models) from the first dir that
    /// has both files: $ARROWHEAD_MODELS, `models/` next to the exe, /usr/share/arrowhead/models,
    /// /usr/lib/arrowhead/models, ~/.cache/ocrs. Other sets live in `<dir>/<model>/`.
    pub fn load(model: &str) -> Result<Self, String> {
        let dir = find_models(model).ok_or_else(|| {
            format!(
                "OCR model set '{model}' not found ({DET} and {REC}); searched {:?}",
                search_dirs(model)
            )
        })?;
        Self::load_from(&dir)
    }

    pub fn load_from(dir: &Path) -> Result<Self, String> {
        if dir.join("dict.txt").is_file() {
            return Ok(Self {
                engine: Engine::Paddle(Box::new(paddle::Paddle::load_from(dir)?)),
            });
        }
        let m = |f: &str| Model::load_file(dir.join(f)).map_err(|e| format!("{f}: {e}"));
        let engine = OcrEngine::new(OcrEngineParams {
            detection_model: Some(m(DET)?),
            recognition_model: Some(m(REC)?),
            ..Default::default()
        })
        .map_err(|e| e.to_string())?;
        Ok(Self {
            engine: Engine::Ocrs(engine),
        })
    }

    /// Text lines in `f` with their rects in stream pixels.
    pub fn lines(&self, f: &FrameView, variant: Variant) -> Vec<(String, Rect)> {
        if f.region.w == 0 || f.region.h == 0 {
            return Vec::new();
        }
        let (rgb, sw, sh, scale) = preprocess(f, variant);
        self.run(&rgb, sw, sh, scale, f.region).unwrap_or_default()
    }

    fn run(
        &self,
        rgb: &[u8],
        w: usize,
        h: usize,
        scale: usize,
        region: Rect,
    ) -> Result<Vec<(String, Rect)>, Box<dyn Error>> {
        let engine = match &self.engine {
            Engine::Ocrs(e) => e,
            Engine::Paddle(p) => {
                return Ok(p
                    .read(rgb, w, h)?
                    .into_iter()
                    .map(|(t, b)| (t, map_rect(b.0, b.1, b.2, b.3, scale, region)))
                    .collect());
            }
        };
        let input = engine.prepare_input(ImageSource::from_bytes(rgb, (w as u32, h as u32))?)?;
        let words = engine.detect_words(&input)?;
        let lines = engine.find_text_lines(&input, &words);
        let mut out = Vec::new();
        for l in engine.recognize_text(&input, &lines)?.into_iter().flatten() {
            let r = l.bounding_rect();
            out.push((
                l.to_string(),
                map_rect(r.left(), r.top(), r.width(), r.height(), scale, region),
            ));
        }
        Ok(out)
    }
}

/// Rect in the upscaled image back to stream coordinates.
fn map_rect(x: i32, y: i32, w: i32, h: i32, scale: usize, region: Rect) -> Rect {
    let s = scale as i32;
    // Clamped: the models pad their boxes a few pixels past the text.
    let (x0, y0) = (x.div_euclid(s).max(0), y.div_euclid(s).max(0));
    let x1 = ((x + w + s - 1).div_euclid(s)).min(region.w as i32);
    let y1 = ((y + h + s - 1).div_euclid(s)).min(region.h as i32);
    Rect::new(
        region.x + x0,
        region.y + y0,
        (x1 - x0).max(0) as u32,
        (y1 - y0).max(0) as u32,
    )
}

/// RGB bytes of the frame, upscaled; returns (rgb, w, h, scale).
fn preprocess(f: &FrameView, v: Variant) -> (Vec<u8>, usize, usize, usize) {
    let (w, h) = (f.region.w as usize, f.region.h as usize);
    let mut rgb = Vec::with_capacity(w * h * 3);
    for y in 0..h {
        for x in 0..w {
            let i = y * f.stride + x * 4;
            let (b, g, r) = (f.data[i], f.data[i + 1], f.data[i + 2]);
            if v == Variant::Threshold2x {
                let l = (299 * r as u32 + 587 * g as u32 + 114 * b as u32) / 1000;
                let o = if l > 140 { 0 } else { 255 };
                rgb.extend([o, o, o]);
            } else {
                rgb.extend([r, g, b]);
            }
        }
    }
    let (scale, lanczos) = match v {
        Variant::Plain2x | Variant::Threshold2x => (2, false),
        Variant::Lanczos3x => (3, true),
    };
    (
        resize(&rgb, w, h, scale, lanczos),
        w * scale,
        h * scale,
        scale,
    )
}

fn kernel(x: f32, lanczos: bool) -> f32 {
    let x = x.abs();
    if !lanczos {
        return (1.0 - x).max(0.0);
    }
    if x < 1e-6 {
        return 1.0;
    }
    if x >= 3.0 {
        return 0.0;
    }
    let p = std::f32::consts::PI * x;
    3.0 * p.sin() * (p / 3.0).sin() / (p * p)
}

/// For each output index along one axis: (source index, normalised weight) pairs.
fn taps(n: usize, scale: usize, lanczos: bool) -> Vec<Vec<(usize, f32)>> {
    let radius = if lanczos { 3.0 } else { 1.0 };
    (0..n * scale)
        .map(|o| {
            let c = (o as f32 + 0.5) / scale as f32 - 0.5;
            let mut ws: Vec<(usize, f32)> = ((c - radius).ceil() as i64
                ..=(c + radius).floor() as i64)
                .map(|s| {
                    (
                        s.clamp(0, n as i64 - 1) as usize,
                        kernel(s as f32 - c, lanczos),
                    )
                })
                .collect();
            let sum: f32 = ws.iter().map(|t| t.1).sum();
            ws.iter_mut().for_each(|t| t.1 /= sum);
            ws
        })
        .collect()
}

fn resize(src: &[u8], w: usize, h: usize, s: usize, lanczos: bool) -> Vec<u8> {
    let (tx, ty) = (taps(w, s, lanczos), taps(h, s, lanczos));
    let ow = w * s;
    let mut mid = vec![0f32; h * ow * 3];
    for y in 0..h {
        for (ox, t) in tx.iter().enumerate() {
            for c in 0..3 {
                mid[(y * ow + ox) * 3 + c] = t
                    .iter()
                    .map(|&(i, k)| src[(y * w + i) * 3 + c] as f32 * k)
                    .sum();
            }
        }
    }
    let mut out = vec![0u8; ow * h * s * 3];
    for (oy, t) in ty.iter().enumerate() {
        for i in 0..ow * 3 {
            let a: f32 = t.iter().map(|&(r, k)| mid[r * ow * 3 + i] * k).sum();
            out[oy * ow * 3 + i] = a.round().clamp(0.0, 255.0) as u8;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(data: &[u8], w: u32, h: u32) -> FrameView<'_> {
        FrameView {
            seq: 0,
            pts: arrowhead_core::time::Nanos(0),
            region: Rect::new(0, 0, w, h),
            stride: w as usize * 4,
            data,
        }
    }

    #[test]
    fn upscale_and_threshold() {
        // 2x1: white then black (BGRx)
        let d = [255, 255, 255, 255, 0, 0, 0, 255];
        let f = frame(&d, 2, 1);
        let (o, w, h, s) = preprocess(&f, Variant::Plain2x);
        assert_eq!((w, h, s, o.len()), (4, 2, 2, 24));
        assert_eq!(&o[0..3], &[255, 255, 255]);
        assert_eq!(&o[9..12], &[0, 0, 0]);
        let (o, ..) = preprocess(&f, Variant::Threshold2x);
        assert_eq!(&o[0..3], &[0, 0, 0]); // bright text -> black
        assert_eq!(&o[9..12], &[255, 255, 255]);
        let (o, w, h, s) = preprocess(&f, Variant::Lanczos3x);
        assert_eq!((w, h, s, o.len()), (6, 3, 3, 54));
        assert_eq!(&o[0..3], &[255, 255, 255]);
    }

    /// Runs the real engine when models are installed (CI has none: skipped).
    #[test]
    fn reads_block_digits() {
        let Ok(ocr) = Ocr::load("default") else {
            eprintln!("skipped: no OCR models");
            return;
        };
        // "2024" in 3x5 block digits, 12 px cells, black on white, at stream offset (100, 50).
        let glyphs: [[&str; 5]; 4] = [
            ["111", "001", "111", "100", "111"], // 2
            ["111", "101", "101", "101", "111"], // 0
            ["111", "001", "111", "100", "111"], // 2
            ["101", "101", "111", "001", "001"], // 4
        ];
        let (cell, w, h) = (12usize, 4 * 4 * 12 + 48, 5 * 12 + 48);
        let mut d = vec![255u8; w * h * 4];
        for (gi, g) in glyphs.iter().enumerate() {
            for (r, row) in g.iter().enumerate() {
                for (c, ch) in row.bytes().enumerate() {
                    if ch == b'1' {
                        for y in 0..cell {
                            for x in 0..cell {
                                let (px, py) = (24 + (gi * 4 + c) * cell + x, 24 + r * cell + y);
                                d[(py * w + px) * 4..][..3].fill(0);
                            }
                        }
                    }
                }
            }
        }
        let f = FrameView {
            seq: 0,
            pts: arrowhead_core::time::Nanos(0),
            region: Rect::new(100, 50, w as u32, h as u32),
            stride: w * 4,
            data: &d,
        };
        for v in [Variant::Plain2x, Variant::Lanczos3x, Variant::Threshold2x] {
            for (_, r) in ocr.lines(&f, v) {
                assert!(
                    r.x >= 100
                        && r.y >= 50
                        && r.x + r.w as i32 <= 100 + w as i32 + 2
                        && r.y + r.h as i32 <= 50 + h as i32 + 2,
                    "{r:?}"
                );
            }
        }
    }

    #[test]
    fn rect_maps_to_stream() {
        assert_eq!(
            map_rect(20, 10, 40, 14, 2, Rect::new(100, 200, 50, 50)),
            Rect::new(110, 205, 20, 7)
        );
        assert_eq!(
            map_rect(10, 10, 10, 10, 3, Rect::new(0, 0, 50, 50)),
            Rect::new(3, 3, 4, 4)
        );
    }
}
