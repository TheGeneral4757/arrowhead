//! OCR worker thread for the HUD thread (spec 03, OCR teacher). An OCR pass takes 0.1-1.5 s,
//! which would stall readback at 20 Hz, so jobs run here one at a time; each job carries the
//! frame copies it is about and its result brings them back, so the HUD thread teaches the
//! glyph reader on exactly the pixels OCR read.

use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, sync_channel};

use arrowhead_core::frame::{OwnedFrame, Rect};
use arrowhead_core::hud::{Counts, ResultsRead, StatsRead};
use arrowhead_ocr::{Ocr, Variant};

use crate::ocrteach;

const VARIANTS: [Variant; 3] = [Variant::Plain2x, Variant::Lanczos3x, Variant::Threshold2x];

pub enum Job {
    /// Find the HUD elements on a full frame. `column` (our playfield, full height) is read
    /// again on its own: the ms readout's thin font is lost in a whole-frame pass.
    Locate {
        frame: OwnedFrame,
        column: Option<Rect>,
    },
    /// Read the stats panel and the counts in every variant.
    Teach {
        stats: OwnedFrame,
        counts: OwnedFrame,
    },
    /// Read our ms readout at `rect` and keep the reading within `tol` of `expected`.
    Ms {
        frame: OwnedFrame,
        rect: Rect,
        expected: f32,
        tol: f32,
    },
    /// Read the song title line.
    Title(OwnedFrame),
    /// Read the results screen ("Stats from Last Song") on a full frame.
    Results(OwnedFrame),
    /// Read both scores (`[left, right]` crops) in every variant.
    Scores([Option<OwnedFrame>; 2]),
}

pub enum Done {
    /// OCR lines and the score regions (`[left, right]`) found from them.
    Lines(Vec<(String, Rect)>, [Option<Rect>; 2]),
    Teach {
        stats: Vec<StatsRead>,
        counts: Vec<Counts>,
        stats_frame: OwnedFrame,
        counts_frame: OwnedFrame,
    },
    Ms {
        value: Option<f32>,
        frame: OwnedFrame,
        rect: Rect,
    },
    Title(Option<String>),
    Results(Option<ResultsRead>),
    /// Scores by screen side, each one only when two OCR passes agree, with the crop it was
    /// read from (to teach the glyph reader).
    Scores([Option<(i32, OwnedFrame)>; 2]),
}

pub struct Worker {
    jobs: SyncSender<Job>,
    done: Receiver<Done>,
    busy: bool,
}

impl Worker {
    /// Loads the OCR models (`ui.ocr_model`) on the worker thread; `Err` if they are missing.
    pub fn start(model: String) -> Result<Self, String> {
        let (jobs, job_rx) = sync_channel::<Job>(1);
        let (done_tx, done) = sync_channel::<Done>(1);
        let (ready_tx, ready) = sync_channel::<Result<(), String>>(1);
        std::thread::Builder::new()
            .name("ocr".into())
            .spawn(move || {
                let ocr = match Ocr::load(&model) {
                    Ok(o) => {
                        let _ = ready_tx.send(Ok(()));
                        o
                    }
                    Err(e) => return drop(ready_tx.send(Err(e))),
                };
                for job in job_rx {
                    if done_tx.send(run(&ocr, job)).is_err() {
                        return;
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        ready.recv().map_err(|e| e.to_string())??;
        Ok(Self {
            jobs,
            done,
            busy: false,
        })
    }

    /// Queues `job` unless one is running. Returns whether it was taken.
    pub fn offer(&mut self, job: Job) -> bool {
        if self.busy {
            return false;
        }
        self.busy = self.jobs.try_send(job).is_ok();
        self.busy
    }

    pub fn busy(&self) -> bool {
        self.busy
    }

    /// The finished job's result, if there is one.
    pub fn poll(&mut self) -> Option<Done> {
        match self.done.try_recv() {
            Ok(d) => {
                self.busy = false;
                Some(d)
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.busy = true; // never offer again
                None
            }
        }
    }
}

fn run(ocr: &Ocr, job: Job) -> Done {
    match job {
        Job::Locate { frame, column } => {
            let mut lines = ocr.lines(&frame.view(), Variant::Plain2x);
            if let Some(c) = column.and_then(|c| frame.region.intersect(&c)) {
                lines.extend(ocr.lines(&frame.view().crop(c).view(), Variant::Plain2x));
            }
            let scores = ocrteach::score_regions(&lines, &frame.view());
            Done::Lines(lines, scores)
        }
        Job::Teach { stats, counts } => Done::Teach {
            stats: VARIANTS
                .iter()
                .map(|&v| ocrteach::stats(&ocr.lines(&stats.view(), v)))
                .collect(),
            counts: VARIANTS
                .iter()
                .map(|&v| ocrteach::counts(&ocr.lines(&counts.view(), v)))
                .collect(),
            stats_frame: stats,
            counts_frame: counts,
        },
        Job::Ms {
            frame,
            rect,
            expected,
            tol,
        } => {
            let cands: Vec<f32> = VARIANTS
                .iter()
                .flat_map(|&v| ocr.lines(&frame.view(), v))
                .filter_map(|(t, _)| ocrteach::ms_value(&t))
                .collect();
            Done::Ms {
                value: ocrteach::confirm_ms(&cands, expected, tol),
                frame,
                rect,
            }
        }
        Job::Results(f) => {
            // a pass costs a second on a full frame: stop at the first that is enough, then
            // read the counts column again on its own (its digits are lost in the full pass)
            let mut passes = vec![];
            for &v in &VARIANTS {
                passes.push(ocr.lines(&f.view(), v));
                if let Some(r) = ocrteach::results(&passes) {
                    return Done::Results(Some(r));
                }
            }
            let panel = passes
                .iter()
                .find_map(|p| ocrteach::results_panel(p))
                .and_then(|r| f.region.intersect(&r));
            if let Some(c) = panel {
                let col = f.view().crop(c);
                passes.extend(VARIANTS.iter().map(|&v| ocr.lines(&col.view(), v)));
            }
            Done::Results(ocrteach::results(&passes))
        }
        Job::Scores(crops) => Done::Scores(crops.map(|c| {
            let c = c?;
            let f = c.view();
            let cands: Vec<i32> = VARIANTS
                .iter()
                .filter_map(|&v| ocrteach::score_value(&ocr.lines(&f, v)))
                .filter_map(|v| {
                    std::iter::once(v)
                        .chain(ocrteach::minus_one_repairs(v))
                        .find(|&v| ocrteach::score_fits(&f, v))
                })
                .collect();
            Some((ocrteach::confirm_score(&cands)?, c))
        })),
        Job::Title(f) => Done::Title(
            ocr.lines(&f.view(), Variant::Plain2x)
                .into_iter()
                .min_by_key(|(_, r)| r.y)
                .map(|(t, _)| t),
        ),
    }
}

/// The whole OCR path on a real frame of the user's game, when this machine has both the
/// models and the frame (local only: frames are never committed, CI has no models).
#[cfg(test)]
mod tests {
    use super::*;
    use arrowhead_core::frame::OwnedFrame;
    use arrowhead_core::hud::{Atlas, HudReader, Layout};
    use arrowhead_core::time::Nanos;

    fn load_png(p: &std::path::Path) -> Option<OwnedFrame> {
        let mut dec = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(p).ok()?));
        dec.set_transformations(png::Transformations::EXPAND);
        let mut r = dec.read_info().ok()?;
        let mut buf = vec![0; r.output_buffer_size()?];
        let info = r.next_frame(&mut buf).ok()?;
        let n = info.color_type.samples();
        let data = buf[..info.buffer_size()]
            .chunks_exact(n)
            .flat_map(|px| [px[2], px[1], px[0], 255])
            .collect();
        Some(OwnedFrame {
            seq: 0,
            pts: Nanos(0),
            region: Rect::new(0, 0, info.width, info.height),
            data,
        })
    }

    fn wait(w: &mut Worker) -> Done {
        loop {
            if let Some(d) = w.poll() {
                return d;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    /// `ARROWHEAD_TEST_SCORES`: `path.png=left,right;...` (the user's frames, local only).
    #[test]
    fn ocr_reads_scores_on_real_frames() {
        let (Some(spec), true) = (
            std::env::var("ARROWHEAD_TEST_SCORES").ok(),
            arrowhead_ocr::find_models("default").is_some(),
        ) else {
            eprintln!("skipped: set ARROWHEAD_TEST_SCORES and have the OCR models");
            return;
        };
        let mut w = Worker::start("default".into()).unwrap();
        let mut rd = HudReader::new(Atlas::default(), 0.25);
        let mut seen = vec![];
        for item in spec.split(';') {
            let (path, want) = item.split_once('=').unwrap();
            let want: Vec<i32> = want.split(',').map(|v| v.parse().unwrap()).collect();
            let frame = load_png(std::path::Path::new(path)).unwrap();
            assert!(w.offer(Job::Locate {
                frame: frame.clone(),
                column: None,
            }));
            let Done::Lines(_, regions) = wait(&mut w) else {
                panic!()
            };
            assert!(w.offer(Job::Scores(
                regions.map(|r| r.map(|r| frame.view().crop(r)))
            )));
            let Done::Scores(got) = wait(&mut w) else {
                panic!()
            };
            eprintln!("{path}: OCR scores {got:?}");
            for i in 0..2 {
                // OCR may give up, never misread
                let got = got[i].as_ref().map(|g| g.0);
                assert!(got.is_none() || got == Some(want[i]), "{path}: {got:?}");
                if let Some(v) = got {
                    assert!(rd.teach_score_at(&frame.view(), regions[i].unwrap(), v));
                }
            }
            seen.push((frame, regions, want));
        }
        // taught by OCR, the glyph reader reads every frame; unknown digits read as nothing
        let mut read = 0;
        for (frame, regions, want) in &seen {
            for i in 0..2 {
                let g = rd.read_score(&frame.view(), regions[i].unwrap());
                eprintln!("glyph {:?} want {}", g, want[i]);
                assert!(g.is_none() || g == Some(want[i]));
                read += g.is_some() as u32;
            }
        }
        assert!(read as usize >= seen.len(), "glyph reads {read}");
    }

    #[test]
    fn ocr_reads_a_results_screen() {
        let (Some(p), true) = (
            std::env::var_os("ARROWHEAD_TEST_RESULTS"),
            arrowhead_ocr::find_models("default").is_some(),
        ) else {
            eprintln!("skipped: set ARROWHEAD_TEST_RESULTS to a results screenshot");
            return;
        };
        let frame = load_png(std::path::Path::new(&p)).unwrap();
        let mut w = Worker::start("default".into()).unwrap();
        assert!(w.offer(Job::Results(frame)));
        let Done::Results(r) = wait(&mut w) else {
            panic!()
        };
        eprintln!("results {r:?}");
        assert!(r.is_some());
    }

    #[test]
    fn ocr_locates_reads_title_and_teaches_on_a_real_frame() {
        let frame = std::env::var_os("ARROWHEAD_TEST_FRAME")
            .map(std::path::PathBuf::from)
            .and_then(|p| load_png(&p));
        let (Some(frame), true) = (frame, arrowhead_ocr::find_models("default").is_some()) else {
            eprintln!("skipped: set ARROWHEAD_TEST_FRAME to a game frame and have the OCR models");
            return;
        };
        let mut w = Worker::start("default".into()).unwrap();
        assert!(w.offer(Job::Locate {
            frame: frame.clone(),
            column: Some(Rect::new(870, 0, 820, 1440)),
        }));
        let Done::Lines(lines, _) = wait(&mut w) else {
            panic!()
        };
        // our playfield on the user's 2560x1440 screen (test only)
        let found = ocrteach::locate(&lines, Some((870, 1690)), 2560);
        eprintln!("found {found:?}");
        let (sr, cr, tr) = (
            found.stats.unwrap(),
            found.counts.unwrap(),
            found.title.unwrap(),
        );
        assert!(found.ms.is_some(), "our ms readout");

        assert!(w.offer(Job::Title(frame.view().crop(tr))));
        let Done::Title(t) = wait(&mut w) else {
            panic!()
        };
        eprintln!("title {t:?}");
        assert!(t.unwrap().to_uppercase().contains("ACCELERANT"));

        assert!(w.offer(Job::Teach {
            stats: frame.view().crop(sr),
            counts: frame.view().crop(cr),
        }));
        let Done::Teach {
            stats,
            counts,
            stats_frame,
            counts_frame,
        } = wait(&mut w)
        else {
            panic!()
        };
        let (st, cn) = ocrteach::trusted(&stats, &counts);
        eprintln!("trusted {st:?} {cn:?}");
        let mut rd = HudReader::new(Atlas::default(), 0.25);
        let mut taught = 0;
        for (i, v) in [
            st.hits.map(|v| v.to_string()),
            st.kps.map(|v| v.to_string()),
        ]
        .iter()
        .zip([3, 6])
        .filter_map(|(v, i)| v.as_ref().map(|v| (i, v)))
        {
            taught += rd.teach_row(&stats_frame.view(), Layout::Stats, sr, i, v) as u32;
        }
        for (i, v) in [cn.sick, cn.good, cn.ok, cn.bad].iter().enumerate() {
            if let Some(v) = v {
                taught += rd.teach_row(&counts_frame.view(), Layout::Counts, cr, i, &v.to_string())
                    as u32;
            }
        }
        eprintln!(
            "taught {taught} rows, stats font knows {:?}",
            rd.atlas.chars(arrowhead_core::hud::Face::Stats)
        );
        // what was taught now reads back with the fast glyph reader, no OCR
        let back = rd.read_counts(&counts_frame.view(), cr);
        assert_eq!(
            (back.sick, back.good, back.ok, back.bad),
            (cn.sick, cn.good, cn.ok, cn.bad)
        );
        assert!(taught > 0);
    }
}
