//! PP-OCR (PaddleOCR) detection + CTC recognition on `rten`. Used for the optional larger
//! model set: a dir with `text-detection.onnx`, `text-recognition.onnx` and `dict.txt`.

use std::error::Error;
use std::path::Path;

use rten::Model;
use rten_tensor::Tensor;
use rten_tensor::prelude::*;

pub struct Paddle {
    det: Model,
    rec: Model,
    dict: Vec<String>,
}

type Box4 = (i32, i32, i32, i32); // x, y, w, h

impl Paddle {
    pub fn load_from(dir: &Path) -> Result<Self, String> {
        let m = |f: &str| Model::load_file(dir.join(f)).map_err(|e| format!("{f}: {e}"));
        let dict =
            std::fs::read_to_string(dir.join("dict.txt")).map_err(|e| format!("dict.txt: {e}"))?;
        Ok(Self {
            det: m("text-detection.onnx")?,
            rec: m("text-recognition.onnx")?,
            dict: dict.lines().map(str::to_owned).collect(),
        })
    }

    /// Text lines of an RGB image, as (text, x, y, w, h) in that image's pixels.
    pub fn read(
        &self,
        rgb: &[u8],
        w: usize,
        h: usize,
    ) -> Result<Vec<(String, Box4)>, Box<dyn Error>> {
        let mut boxes = self.detect(rgb, w, h)?;
        boxes.sort_by_key(|b| (b.1 + b.3 / 2) / 8 * 100_000 + b.0);
        let mut out = Vec::new();
        for b in boxes {
            let t = self.recognise(rgb, w, h, b)?;
            if !t.is_empty() {
                out.push((t, b));
            }
        }
        Ok(out)
    }

    fn detect(&self, rgb: &[u8], w: usize, h: usize) -> Result<Vec<Box4>, Box<dyn Error>> {
        // Cap the longest side: the DB model's memory grows with the image.
        let k = (w.max(h) as f32 / 1920.0).ceil().max(1.0) as usize;
        let (dw, dh) = (w / k, h / k);
        let (pw, ph) = (dw.div_ceil(32) * 32, dh.div_ceil(32) * 32);
        let (mean, std) = ([0.485f32, 0.456, 0.406], [0.229f32, 0.224, 0.225]);
        let mut data = vec![0f32; 3 * ph * pw];
        for y in 0..dh {
            for x in 0..dw {
                for c in 0..3 {
                    let v = rgb[((y * k) * w + x * k) * 3 + c] as f32 / 255.0;
                    data[c * ph * pw + y * pw + x] = (v - mean[c]) / std[c];
                }
            }
        }
        let input = Tensor::from_data(&[1, 3, ph, pw], data);
        let out: Tensor<f32> = self.det.run_one(input.view().into(), None)?.try_into()?;
        let p = out.data().ok_or("det output not contiguous")?;
        let mut seen = vec![false; ph * pw];
        let mut boxes = Vec::new();
        let mut stack = Vec::new();
        for s in 0..ph * pw {
            if seen[s] || p[s] <= 0.3 {
                continue;
            }
            let (mut x0, mut y0, mut x1, mut y1, mut area) = (usize::MAX, usize::MAX, 0, 0, 0usize);
            seen[s] = true;
            stack.push(s);
            while let Some(i) = stack.pop() {
                let (x, y) = (i % pw, i / pw);
                (x0, y0, x1, y1, area) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y), area + 1);
                for (nx, ny) in [
                    (x.wrapping_sub(1), y),
                    (x + 1, y),
                    (x, y.wrapping_sub(1)),
                    (x, y + 1),
                ] {
                    if nx < pw && ny < ph && !seen[ny * pw + nx] && p[ny * pw + nx] > 0.3 {
                        seen[ny * pw + nx] = true;
                        stack.push(ny * pw + nx);
                    }
                }
            }
            let (bw, bh) = ((x1 - x0 + 1) as f32, (y1 - y0 + 1) as f32);
            if bw.min(bh) < 3.0 || area < 10 {
                continue;
            }
            // DB unclip: grow by area * 1.5 / perimeter.
            let d = (bw * bh * 1.5 / (2.0 * (bw + bh))).round() as i32;
            let (x, y) = (
                (x0 as i32 - d).max(0) * k as i32,
                (y0 as i32 - d).max(0) * k as i32,
            );
            let (ex, ey) = (
                ((x1 as i32 + d + 1) * k as i32).min(w as i32),
                ((y1 as i32 + d + 1) * k as i32).min(h as i32),
            );
            boxes.push((x, y, ex - x, ey - y));
        }
        Ok(boxes)
    }

    fn recognise(&self, rgb: &[u8], w: usize, h: usize, b: Box4) -> Result<String, Box<dyn Error>> {
        const H: usize = 48;
        let (cw, ch) = (b.2 as usize, b.3 as usize);
        let nw = (H * cw).div_ceil(ch).clamp(16, 3200);
        let mut data = vec![0f32; 3 * H * nw];
        for y in 0..H {
            let sy = ((y as f32 + 0.5) * ch as f32 / H as f32 - 0.5).clamp(0.0, ch as f32 - 1.0);
            let (y0, fy) = (sy as usize, sy.fract());
            for x in 0..nw {
                let sx =
                    ((x as f32 + 0.5) * cw as f32 / nw as f32 - 0.5).clamp(0.0, cw as f32 - 1.0);
                let (x0, fx) = (sx as usize, sx.fract());
                let px = |xx: usize, yy: usize, c: usize| {
                    let (xx, yy) = (
                        (b.0 as usize + xx.min(cw - 1)).min(w - 1),
                        (b.1 as usize + yy.min(ch - 1)).min(h - 1),
                    );
                    rgb[(yy * w + xx) * 3 + c] as f32
                };
                for c in 0..3 {
                    let top = px(x0, y0, c) * (1.0 - fx) + px(x0 + 1, y0, c) * fx;
                    let bot = px(x0, y0 + 1, c) * (1.0 - fx) + px(x0 + 1, y0 + 1, c) * fx;
                    data[c * H * nw + y * nw + x] = (top * (1.0 - fy) + bot * fy) / 127.5 - 1.0;
                }
            }
        }
        let input = Tensor::from_data(&[1, 3, H, nw], data);
        let out: Tensor<f32> = self.rec.run_one(input.view().into(), None)?.try_into()?;
        let (t, c) = (out.shape()[1], out.shape()[2]);
        let p = out.data().ok_or("rec output not contiguous")?;
        let (mut s, mut last) = (String::new(), 0usize);
        for i in 0..t {
            let row = &p[i * c..(i + 1) * c];
            let k = (0..c)
                .max_by(|&a, &b| row[a].total_cmp(&row[b]))
                .unwrap_or(0);
            if k != 0 && k != last {
                match self.dict.get(k - 1) {
                    Some(ch) => s.push_str(ch),
                    None => s.push(' '), // the extra class after the dict is the space
                }
            }
            last = k;
        }
        Ok(s)
    }
}
