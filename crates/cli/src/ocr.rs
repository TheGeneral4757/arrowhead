//! `cli ocr`: run the OCR teacher on PNGs and print what it reads.

use std::path::PathBuf;
use std::time::Instant;

use arrowhead_ocr::{Ocr, Variant};

use crate::Res;

pub fn run(files: &[PathBuf], variant: &str, model: &str) -> Res {
    let vs: Vec<(&str, Variant)> = match variant {
        "plain" => vec![("plain", Variant::Plain2x)],
        "lanczos" => vec![("lanczos", Variant::Lanczos3x)],
        "threshold" => vec![("threshold", Variant::Threshold2x)],
        "all" => vec![
            ("plain", Variant::Plain2x),
            ("lanczos", Variant::Lanczos3x),
            ("threshold", Variant::Threshold2x),
        ],
        v => return Err(format!("unknown variant '{v}' (plain|lanczos|threshold|all)").into()),
    };
    let t = Instant::now();
    let ocr = Ocr::load(model)?;
    println!(
        "model '{model}' loaded in {:.0} ms",
        t.elapsed().as_secs_f64() * 1e3
    );
    for f in files {
        let fr = crate::hud::load_png(f)?;
        println!("== {} ({}x{})", f.display(), fr.region.w, fr.region.h);
        for (name, v) in &vs {
            let t = Instant::now();
            let lines = ocr.lines(&fr.view(), *v);
            println!(
                "-- {name}: {} lines, {:.0} ms",
                lines.len(),
                t.elapsed().as_secs_f64() * 1e3
            );
            for (s, r) in lines {
                println!("  {s:?} @ {},{} {}x{}", r.x, r.y, r.w, r.h);
            }
        }
    }
    Ok(())
}
