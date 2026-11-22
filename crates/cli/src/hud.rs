//! `cli hud`: teach the glyph atlas from known text on screenshots, then locate and read.

use std::path::{Path, PathBuf};

use arrowhead_core::frame::{OwnedFrame, Rect};
use arrowhead_core::hud::{Atlas, Face, HudReader, Layout};
use arrowhead_core::time::Nanos;

use crate::Res;

fn load_png(p: &Path) -> Result<OwnedFrame, Box<dyn std::error::Error>> {
    let mut dec = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(p)?));
    dec.set_transformations(png::Transformations::EXPAND);
    let mut r = dec.read_info()?;
    let mut buf = vec![0; r.output_buffer_size().ok_or("png too large")?];
    let info = r.next_frame(&mut buf)?;
    let n = info.color_type.samples();
    let (w, h) = (info.width as usize, info.height as usize);
    let mut data = Vec::with_capacity(w * h * 4);
    for px in buf[..info.buffer_size()].chunks_exact(n) {
        let (r, g, b) = if n >= 3 {
            (px[0], px[1], px[2])
        } else {
            (px[0], px[0], px[0])
        };
        data.extend([b, g, r, 255]);
    }
    Ok(OwnedFrame {
        seq: 0,
        pts: Nanos(0),
        region: Rect::new(0, 0, w as u32, h as u32),
        data,
    })
}

/// `file.png:kind=v1|v2|...`; kinds: stats, counts, rstats, rcounts (values per row, top to
/// bottom), ms (one text), score (left,right); the last two take a `@x,y,w,h` search hint.
fn teach(rd: &mut HudReader, dir: &Path, spec: &str) -> Res {
    let (file, rest) = spec.split_once(':').ok_or("teach: file.png:kind=values")?;
    let (kind, vals) = rest.split_once('=').ok_or("teach: kind=values")?;
    // optional `@x,y,w,h` search hint for ms and score
    let (vals, hint) = match vals.split_once('@') {
        Some((v, h)) => (v, Some(crate::parse_rect(h)?)),
        None => (vals, None),
    };
    let fr = load_png(&dir.join(file))?;
    let v = fr.view();
    let rows: Vec<&str> = vals.split('|').collect();
    let res = match kind {
        "stats" => rd.teach_stack(&v, Layout::Stats, &rows),
        "counts" => rd.teach_stack(&v, Layout::Counts, &rows),
        "rstats" => rd.teach_stack(&v, Layout::ResStats, &rows),
        "rcounts" => rd.teach_stack(&v, Layout::ResCounts, &rows),
        "ms" => rd.teach_lines(&v, Face::Ms, &[vals], hint),
        "score" => rd.teach_lines(&v, Face::Score, &vals.split(',').collect::<Vec<_>>(), hint),
        k => Err(format!("unknown teach kind {k}")),
    };
    res.map_err(|e| format!("teach {spec}: {e}").into())
}

pub fn run(
    files: &[PathBuf],
    teaches: &[String],
    dir: &Path,
    atlas: Option<&Path>,
    save: bool,
    max_sad: f32,
) -> Res {
    let mut rd = HudReader::new(atlas.map(Atlas::load).unwrap_or_default(), max_sad);
    for t in teaches {
        teach(&mut rd, dir, t)?;
    }
    if let (true, Some(p)) = (save, atlas) {
        rd.atlas.save(p)?;
        println!("atlas saved to {}", p.display());
    }
    println!(
        "atlas: stats '{}' ms '{}' score '{}' results '{}'",
        rd.atlas.chars(Face::Stats),
        rd.atlas.chars(Face::Ms),
        rd.atlas.chars(Face::Score),
        rd.atlas.chars(Face::Results)
    );
    for f in files {
        let fr = load_png(f)?;
        let v = fr.view();
        let t = std::time::Instant::now();
        let loc = rd.locate(&v);
        let t_loc = t.elapsed();
        let t = std::time::Instant::now();
        let read = rd.read(&v);
        let t_read = t.elapsed();
        println!("== {} (locate {:?}, read {:?})", f.display(), t_loc, t_read);
        println!("  regions {loc:?}");
        println!("  {read:?}");
        if let Some(r) = rd.read_results(&v) {
            println!("  results {r:?}");
        }
    }
    Ok(())
}

/// Real screenshots stay out of the repo: read from `~/ah-clips`, skip when absent.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_screenshots() {
        let Some(dir) = std::env::var_os("HOME").map(|h| PathBuf::from(h).join("ah-clips")) else {
            return;
        };
        if !dir.join("game6.png").exists() || !dir.join("game2.png").exists() {
            eprintln!("skipped: no ~/ah-clips");
            return;
        }
        let (l, r) = ("@100,1330,240,100", "@2150,1330,320,100");
        let mut teaches: Vec<String> = vec![];
        for (f, m, ms, sl, sr) in [
            ("game2", 135, "-143.26", "-3998", "11377"),
            ("game3", 285, "-145.62", "-8539", "23850"),
            ("game4", 341, "-145.32", "-10230", "32794"),
            ("game5", 417, "-143.85", "-12476", "42022"),
        ] {
            teaches.push(format!("{f}.png:stats=0.00%|0|0|0|{m}|0.00ms|0"));
            teaches.push(format!("{f}.png:ms={ms}ms"));
            teaches.push(format!("{f}.png:score={sl}{l}"));
            teaches.push(format!("{f}.png:score={sr}{r}"));
        }
        teaches.push("game2.png:counts=0|0|0|0|135".into());
        let mut rd = HudReader::new(Atlas::default(), 0.25);
        for t in &teaches {
            teach(&mut rd, &dir, t).unwrap();
        }
        // held out: game6 (-141.65 ms, scores -19543 / 63082), game1 (15 misses, no ms readout)
        let fr = load_png(&dir.join("game6.png")).unwrap();
        let v = fr.view();
        rd.locate(&v);
        let read = rd.read(&v);
        assert_eq!(read.ms, Some(-141.65));
        assert_eq!(read.scores, [Some(-19543), Some(63082)]);
        let fr = load_png(&dir.join("game1.png")).unwrap();
        let v = fr.view();
        rd.locate(&v);
        let read = rd.read(&v);
        assert_eq!(read.stats.unwrap().misses, Some(15));
        assert_eq!(read.counts.unwrap().miss, Some(15));
        assert_eq!(read.ms, None);
        assert_eq!(read.scores, [Some(-395), Some(0)]);
    }
}
