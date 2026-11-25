//! Flight recorder (spec 13, 3.2; `[debug] flight_recorder`): the last 10 s of playfield frames
//! (the scan thread's own frames) and the planned and sent presses, kept compressed on their
//! own thread. When the HUD sees the Misses counter rise, they are written to
//! `<data>/flight/<time>.ahclip` and `<time>.presses.tsv`, which `detect --profile --plan`
//! replays side by side. The scan thread only copies each frame into a recycled buffer and
//! never waits: with no free buffer the frame is skipped (and counted).

use std::collections::VecDeque;
use std::io::Write;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::time::{Instant, SystemTime};

use arrowhead_capture::clip::{CLIP_VERSION, ClipHeader, ClipWriter};
use arrowhead_core::event::Level;
use arrowhead_core::frame::{FrameView, OwnedFrame, Rect};
use arrowhead_core::time::Nanos;

use crate::{Tx, log};

/// Seconds of history kept.
const KEEP_MS: f64 = 10_000.0;
/// Buffers in flight between the scan thread and the recorder.
const POOL: usize = 4;
/// Dumps per run, and the least time between two (one bad stretch gives one clip).
const MAX_DUMPS: u32 = 20;
const DUMP_GAP_S: u64 = 10;
/// Fast zstd: the recorder must keep up with 165 Hz.
const LEVEL: i32 = -3;

pub enum Msg {
    Frame(OwnedFrame),
    Planned {
        id: u32,
        lane: u8,
        at: Nanos,
        release: Nanos,
    },
    Sent {
        id: u32,
        lane: u8,
        planned: Nanos,
        actual: Nanos,
    },
    /// The HUD's Misses counter went from `from` to `to`.
    Missed {
        from: u32,
        to: u32,
    },
}

/// The scan thread's end.
pub struct Flight {
    tx: SyncSender<Msg>,
    pool: Receiver<Vec<u8>>,
    pub skipped: u64,
}

/// Stream facts for the clip header.
pub struct Stream {
    pub width: u32,
    pub height: u32,
    pub refresh_hz: f32,
    pub backend: String,
}

impl Flight {
    /// Starts the recorder thread; returns the scan thread's end and a sender for the HUD.
    pub fn start(stream: Stream, tx: Tx) -> std::io::Result<(Self, SyncSender<Msg>)> {
        let (mtx, rx) = mpsc::sync_channel::<Msg>(512);
        let (back, pool) = mpsc::channel::<Vec<u8>>();
        for _ in 0..POOL {
            let _ = back.send(Vec::new());
        }
        std::thread::Builder::new()
            .name("flight".into())
            .spawn(move || record(rx, back, stream, tx))?;
        Ok((
            Self {
                tx: mtx.clone(),
                pool,
                skipped: 0,
            },
            mtx,
        ))
    }

    /// Copies `f` into a free buffer and hands it over; skips it when none is free.
    pub fn frame(&mut self, f: &FrameView) {
        let Ok(mut data) = self.pool.try_recv() else {
            self.skipped += 1;
            return;
        };
        data.clear();
        let row = f.region.w as usize * 4;
        for r in 0..f.region.h as usize {
            data.extend_from_slice(&f.data[r * f.stride..r * f.stride + row]);
        }
        let msg = Msg::Frame(OwnedFrame {
            seq: f.seq,
            pts: f.pts,
            region: f.region,
            data,
        });
        if self.tx.try_send(msg).is_err() {
            self.skipped += 1;
        }
    }

    pub fn send(&self, m: Msg) {
        let _ = self.tx.try_send(m);
    }
}

struct Kept {
    pts: Nanos,
    seq: u64,
    z: Vec<u8>,
}

fn record(rx: Receiver<Msg>, back: mpsc::Sender<Vec<u8>>, stream: Stream, tx: Tx) {
    let mut frames: VecDeque<Kept> = VecDeque::new();
    let mut region = Rect::new(0, 0, 0, 0);
    let mut events: VecDeque<(Nanos, String)> = VecDeque::new();
    let (mut dumps, mut last_dump) = (0u32, None::<Instant>);
    for m in rx {
        match m {
            Msg::Frame(f) => {
                if f.region != region {
                    // the playfield moved (recalibration): what came before cannot share a clip
                    frames.clear();
                    region = f.region;
                }
                if let Ok(z) = zstd::bulk::compress(&f.data, LEVEL) {
                    frames.push_back(Kept {
                        pts: f.pts,
                        seq: f.seq,
                        z,
                    });
                }
                let _ = back.send(f.data);
                let newest = f.pts;
                while frames
                    .front()
                    .is_some_and(|k| newest.ms_since(k.pts) > KEEP_MS)
                {
                    frames.pop_front();
                }
                while events
                    .front()
                    .is_some_and(|e| newest.ms_since(e.0) > KEEP_MS + 1000.0)
                {
                    events.pop_front();
                }
            }
            Msg::Planned {
                id,
                lane,
                at,
                release,
            } => events.push_back((
                at,
                format!("planned\t{id}\t{lane}\t{:.3}\t{:.3}", at.ms(), release.ms()),
            )),
            Msg::Sent {
                id,
                lane,
                planned,
                actual,
            } => events.push_back((
                actual,
                format!(
                    "sent\t{id}\t{lane}\t{:.3}\t{:.3}",
                    planned.ms(),
                    actual.ms()
                ),
            )),
            Msg::Missed { from, to } => {
                if dumps >= MAX_DUMPS
                    || last_dump.is_some_and(|t| t.elapsed().as_secs() < DUMP_GAP_S)
                    || frames.is_empty()
                {
                    continue;
                }
                dumps += 1;
                last_dump = Some(Instant::now());
                match dump(&frames, region, &events, &stream, (from, to)) {
                    Ok(p) => log(
                        &tx,
                        Level::Info,
                        format!(
                            "flight recorder: misses {from} -> {to}, last {} frames saved to {}",
                            frames.len(),
                            p.display()
                        ),
                    ),
                    Err(e) => log(&tx, Level::Warn, format!("flight recorder: {e}")),
                }
            }
        }
    }
}

fn dump(
    frames: &VecDeque<Kept>,
    region: Rect,
    events: &VecDeque<(Nanos, String)>,
    stream: &Stream,
    (from, to): (u32, u32),
) -> std::io::Result<PathBuf> {
    let dir = arrowhead_core::config::data_dir()
        .ok_or_else(|| std::io::Error::other("no data dir"))?
        .join("flight");
    std::fs::create_dir_all(&dir)?;
    let now = SystemTime::now();
    let stamp = crate::hud::rfc3339(now).replace([':', '-'], "");
    let clip = dir.join(format!("{stamp}.ahclip"));
    let header = ClipHeader {
        version: CLIP_VERSION,
        width: stream.width,
        height: stream.height,
        region,
        refresh_hz: stream.refresh_hz,
        backend: format!("flight recorder ({})", stream.backend),
        recorded_at: crate::hud::rfc3339(now),
        profile_snapshot: None,
        notes: format!("misses {from} -> {to}"),
    };
    let mut w = ClipWriter::create(&clip, &header)?;
    for k in frames {
        w.push_compressed(k.pts, k.seq, &k.z)?;
    }
    w.finish()?;
    let mut t = std::io::BufWriter::new(std::fs::File::create(
        dir.join(format!("{stamp}.presses.tsv")),
    )?);
    writeln!(
        t,
        "# frames {} to {} ms (pts); misses {from} -> {to}",
        frames.front().map_or(0.0, |k| k.pts.ms()),
        frames.back().map_or(0.0, |k| k.pts.ms())
    )?;
    writeln!(
        t,
        "# planned: id lane at_ms release_ms | sent: id lane planned_ms actual_ms"
    )?;
    for (_, l) in events {
        writeln!(t, "{l}")?;
    }
    t.flush()?;
    Ok(clip)
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrowhead_capture::clip::ClipReader;

    #[test]
    fn dump_round_trips_through_a_clip() {
        let region = Rect::new(10, 20, 4, 2);
        let raw = vec![7u8; 4 * 2 * 4];
        let frames: VecDeque<Kept> = (0..3)
            .map(|i| Kept {
                pts: Nanos::from_ms(i as f64 * 6.0),
                seq: i,
                z: zstd::bulk::compress(&raw, LEVEL).unwrap(),
            })
            .collect();
        let dir = std::env::temp_dir().join(format!("ah-flight-{}", std::process::id()));
        let clip = dir.join("t.ahclip");
        std::fs::create_dir_all(&dir).unwrap();
        let mut w = ClipWriter::create(
            &clip,
            &ClipHeader {
                version: CLIP_VERSION,
                width: 64,
                height: 48,
                region,
                refresh_hz: 165.0,
                backend: "t".into(),
                recorded_at: String::new(),
                profile_snapshot: None,
                notes: String::new(),
            },
        )
        .unwrap();
        for k in &frames {
            w.push_compressed(k.pts, k.seq, &k.z).unwrap();
        }
        w.finish().unwrap();
        let mut r = ClipReader::open(&clip).unwrap();
        assert_eq!(r.len(), 3);
        let f = r.read(2).unwrap();
        assert_eq!((f.seq, f.data), (2, raw));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
