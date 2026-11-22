//! `.ahclip`: recorded lane-region frames for tests and replay (spec 02).
//!
//! Layout, little-endian:
//! `u32 header_len`, header JSON, then per frame `u64 pts_ns, u64 seq, u32 len, zstd(BGRx)`,
//! then the index (`u64` file offset per frame), `u64 frame_count`, `b"AHCLIP01"`.

use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use arrowhead_core::frame::{CaptureError, FrameSource, FrameView, OwnedFrame, Rect, SourceInfo};
use arrowhead_core::time::Nanos;
use serde::{Deserialize, Serialize};

const MAGIC: &[u8; 8] = b"AHCLIP01";
pub const CLIP_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClipHeader {
    pub version: u32,
    /// Full stream size.
    pub width: u32,
    pub height: u32,
    /// Recorded region, stream pixels. Every frame is exactly this size.
    pub region: Rect,
    pub refresh_hz: f32,
    pub backend: String,
    pub recorded_at: String,
    pub profile_snapshot: Option<serde_json::Value>,
    pub notes: String,
}

pub struct ClipWriter {
    out: BufWriter<File>,
    pos: u64,
    index: Vec<u64>,
    region: Rect,
    zbuf: Vec<u8>,
}

impl ClipWriter {
    pub fn create(path: &Path, header: &ClipHeader) -> io::Result<Self> {
        let mut out = BufWriter::new(File::create(path)?);
        let json = serde_json::to_vec(header).map_err(io::Error::other)?;
        out.write_all(&(json.len() as u32).to_le_bytes())?;
        out.write_all(&json)?;
        Ok(Self {
            out,
            pos: 4 + json.len() as u64,
            index: Vec::new(),
            region: header.region,
            zbuf: Vec::new(),
        })
    }

    /// Writes one frame. Its region must equal the header's.
    pub fn push(&mut self, f: &FrameView) -> io::Result<()> {
        if f.region != self.region {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "frame region differs from clip region",
            ));
        }
        let row = f.region.w as usize * 4;
        let packed;
        let raw = if f.stride == row {
            &f.data[..row * f.region.h as usize]
        } else {
            packed = f.to_owned().data;
            &packed[..]
        };
        self.zbuf.clear();
        zstd::stream::copy_encode(raw, &mut self.zbuf, 1)?;
        self.index.push(self.pos);
        self.out.write_all(&f.pts.0.to_le_bytes())?;
        self.out.write_all(&f.seq.to_le_bytes())?;
        self.out
            .write_all(&(self.zbuf.len() as u32).to_le_bytes())?;
        self.out.write_all(&self.zbuf)?;
        self.pos += 20 + self.zbuf.len() as u64;
        Ok(())
    }

    pub fn frames(&self) -> usize {
        self.index.len()
    }

    /// Writes the index and trailer. A clip without them cannot be read.
    pub fn finish(mut self) -> io::Result<()> {
        for off in &self.index {
            self.out.write_all(&off.to_le_bytes())?;
        }
        self.out
            .write_all(&(self.index.len() as u64).to_le_bytes())?;
        self.out.write_all(MAGIC)?;
        self.out
            .into_inner()
            .map_err(|e| e.into_error())?
            .sync_all()
    }
}

pub struct ClipReader {
    file: BufReader<File>,
    pub header: ClipHeader,
    index: Vec<u64>,
    zbuf: Vec<u8>,
}

fn bad(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.to_string())
}

impl ClipReader {
    pub fn open(path: &Path) -> io::Result<Self> {
        let mut file = BufReader::new(File::open(path)?);
        let len = file.seek(SeekFrom::End(0))?;
        if len < 20 {
            return Err(bad("not an .ahclip (too short)"));
        }
        let mut tail = [0u8; 16];
        file.seek(SeekFrom::End(-16))?;
        file.read_exact(&mut tail)?;
        if &tail[8..] != MAGIC {
            return Err(bad(
                "not an .ahclip, or recording was not finished (no trailer)",
            ));
        }
        let count = u64::from_le_bytes(tail[..8].try_into().unwrap());
        let index_at = len
            .checked_sub(16 + count * 8)
            .ok_or_else(|| bad("index out of range"))?;
        file.seek(SeekFrom::Start(index_at))?;
        let mut raw = vec![0u8; count as usize * 8];
        file.read_exact(&mut raw)?;
        let index = raw
            .chunks_exact(8)
            .map(|c| u64::from_le_bytes(c.try_into().unwrap()))
            .collect();

        file.seek(SeekFrom::Start(0))?;
        let mut n = [0u8; 4];
        file.read_exact(&mut n)?;
        let mut json = vec![0u8; u32::from_le_bytes(n) as usize];
        file.read_exact(&mut json)?;
        let header: ClipHeader = serde_json::from_slice(&json).map_err(|e| bad(&e.to_string()))?;
        if header.version != CLIP_VERSION {
            return Err(bad(&format!("unsupported clip version {}", header.version)));
        }
        Ok(Self {
            file,
            header,
            index,
            zbuf: Vec::new(),
        })
    }

    pub fn len(&self) -> usize {
        self.index.len()
    }

    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// Decodes frame `i` into `out` (tightly packed BGRx), returns (pts, seq).
    pub fn read_into(&mut self, i: usize, out: &mut Vec<u8>) -> io::Result<(Nanos, u64)> {
        let off = *self
            .index
            .get(i)
            .ok_or_else(|| bad("frame index out of range"))?;
        self.file.seek(SeekFrom::Start(off))?;
        let mut h = [0u8; 20];
        self.file.read_exact(&mut h)?;
        let pts = u64::from_le_bytes(h[..8].try_into().unwrap());
        let seq = u64::from_le_bytes(h[8..16].try_into().unwrap());
        let len = u32::from_le_bytes(h[16..].try_into().unwrap()) as usize;
        self.zbuf.resize(len, 0);
        self.file.read_exact(&mut self.zbuf)?;
        out.clear();
        zstd::stream::copy_decode(&self.zbuf[..], &mut *out)?;
        let r = self.header.region;
        if out.len() != r.w as usize * r.h as usize * 4 {
            return Err(bad("frame size does not match clip region"));
        }
        Ok((Nanos(pts), seq))
    }

    pub fn read(&mut self, i: usize) -> io::Result<OwnedFrame> {
        let mut data = Vec::new();
        let (pts, seq) = self.read_into(i, &mut data)?;
        Ok(OwnedFrame {
            seq,
            pts,
            region: self.header.region,
            data,
        })
    }
}

/// Plays a clip as a `FrameSource`. The end of the clip is reported as `SourceLost`.
pub struct ReplaySource {
    reader: ClipReader,
    next: usize,
    buf: Vec<u8>,
    cur: Option<(Nanos, u64)>,
    region: Rect,
    /// Wall-clock start and first pts, when pacing to the recorded timing.
    pace: Option<(Instant, Nanos)>,
    paced: bool,
}

impl ReplaySource {
    /// `paced`: deliver frames at their recorded intervals; otherwise as fast as asked.
    pub fn open(path: &Path, paced: bool) -> io::Result<Self> {
        let reader = ClipReader::open(path)?;
        let region = reader.header.region;
        Ok(Self {
            reader,
            next: 0,
            buf: Vec::new(),
            cur: None,
            region,
            pace: None,
            paced,
        })
    }

    pub fn header(&self) -> &ClipHeader {
        &self.reader.header
    }

    pub fn len(&self) -> usize {
        self.reader.len()
    }

    pub fn is_empty(&self) -> bool {
        self.reader.is_empty()
    }

    fn view(&self) -> Option<FrameView<'_>> {
        let (pts, seq) = self.cur?;
        let full = self.reader.header.region;
        let r = self.region.intersect(&full).unwrap_or(full);
        let stride = full.w as usize * 4;
        let start = (r.y - full.y) as usize * stride + (r.x - full.x) as usize * 4;
        Some(FrameView {
            seq,
            pts,
            region: r,
            stride,
            data: &self.buf[start..],
        })
    }
}

fn io_err(e: io::Error) -> CaptureError {
    CaptureError::Backend(e.to_string())
}

impl FrameSource for ReplaySource {
    fn next_frame(&mut self, timeout: Duration) -> Result<Option<FrameView<'_>>, CaptureError> {
        if self.next >= self.reader.len() {
            return Err(CaptureError::SourceLost);
        }
        if self.paced {
            let mut hdr = [0u8; 8];
            let off = self.reader.index[self.next];
            self.reader
                .file
                .seek(SeekFrom::Start(off))
                .map_err(io_err)?;
            self.reader.file.read_exact(&mut hdr).map_err(io_err)?;
            let pts = Nanos(u64::from_le_bytes(hdr));
            let (t0, p0) = *self.pace.get_or_insert((Instant::now(), pts));
            let due = t0 + pts.saturating_sub(p0);
            let now = Instant::now();
            if due > now + timeout {
                std::thread::sleep(timeout);
                return Ok(None);
            }
            std::thread::sleep(due.saturating_duration_since(now));
        }
        self.cur = Some(
            self.reader
                .read_into(self.next, &mut self.buf)
                .map_err(io_err)?,
        );
        self.next += 1;
        Ok(self.view())
    }

    fn set_region(&mut self, region: Rect) {
        self.region = region;
    }

    fn info(&self) -> SourceInfo {
        let h = &self.reader.header;
        SourceInfo {
            width: h.width,
            height: h.height,
            refresh_hz: h.refresh_hz,
            backend: format!("replay ({})", h.backend),
            monitor: String::new(),
            scale: 1.0,
            pts_is_receive_time: false,
        }
    }

    fn snapshot(&mut self) -> Result<OwnedFrame, CaptureError> {
        let i = self.next.saturating_sub(1);
        self.reader.read(i).map_err(io_err)
    }
}

/// Records frames on its own thread. `push` never blocks: when the queue is
/// full the frame is dropped and counted. Buffers are recycled, so steady-state
/// recording does not allocate on the caller's thread.
pub struct ClipRecorder {
    tx: Option<SyncSender<OwnedFrame>>,
    pool: Receiver<Vec<u8>>,
    handle: Option<JoinHandle<io::Result<usize>>>,
    pub drops: u64,
}

impl ClipRecorder {
    pub fn start(mut writer: ClipWriter, queue: usize) -> Self {
        let (tx, rx) = mpsc::sync_channel::<OwnedFrame>(queue);
        let (back_tx, pool) = mpsc::channel::<Vec<u8>>();
        let handle = std::thread::Builder::new()
            .name("ah-clip-writer".into())
            .spawn(move || {
                for f in rx {
                    writer.push(&f.view())?;
                    let _ = back_tx.send(f.data);
                }
                let n = writer.frames();
                writer.finish()?;
                Ok(n)
            })
            .expect("spawn clip writer");
        Self {
            tx: Some(tx),
            pool,
            handle: Some(handle),
            drops: 0,
        }
    }

    pub fn push(&mut self, f: &FrameView) {
        let mut data = self.pool.try_recv().unwrap_or_default();
        data.clear();
        let row = f.region.w as usize * 4;
        for r in 0..f.region.h as usize {
            data.extend_from_slice(&f.data[r * f.stride..r * f.stride + row]);
        }
        let frame = OwnedFrame {
            seq: f.seq,
            pts: f.pts,
            region: f.region,
            data,
        };
        if let Some(tx) = &self.tx {
            match tx.try_send(frame) {
                Ok(()) => {}
                Err(TrySendError::Full(_)) => self.drops += 1,
                Err(TrySendError::Disconnected(_)) => self.drops += 1,
            }
        }
    }

    /// Flushes the queue and writes the trailer. Returns frames written.
    pub fn finish(mut self) -> io::Result<usize> {
        drop(self.tx.take());
        self.handle
            .take()
            .unwrap()
            .join()
            .map_err(|_| io::Error::other("clip writer panicked"))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(region: Rect) -> ClipHeader {
        ClipHeader {
            version: CLIP_VERSION,
            width: 64,
            height: 48,
            region,
            refresh_hz: 165.0,
            backend: "test".into(),
            recorded_at: "2026-11-21T00:00:00Z".into(),
            profile_snapshot: None,
            notes: String::new(),
        }
    }

    fn frame(i: u64, region: Rect) -> OwnedFrame {
        let n = region.w as usize * region.h as usize * 4;
        let data = (0..n).map(|b| (b as u64 * 31 + i * 7) as u8).collect();
        OwnedFrame {
            seq: i + 1,
            pts: Nanos(1_000_000_000 + i * 6_060_606),
            region,
            data,
        }
    }

    #[test]
    fn roundtrip_is_bit_exact() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.ahclip");
        let r = Rect::new(10, 5, 8, 6);
        let mut w = ClipWriter::create(&p, &header(r)).unwrap();
        let frames: Vec<_> = (0..20).map(|i| frame(i, r)).collect();
        for f in &frames {
            w.push(&f.view()).unwrap();
        }
        w.finish().unwrap();

        let mut rd = ClipReader::open(&p).unwrap();
        assert_eq!(rd.header, header(r));
        assert_eq!(rd.len(), 20);
        for (i, f) in frames.iter().enumerate() {
            assert_eq!(&rd.read(i).unwrap(), f);
        }
    }

    #[test]
    fn replay_source_yields_frames_then_source_lost() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.ahclip");
        let r = Rect::new(0, 0, 4, 4);
        let mut rec = ClipRecorder::start(ClipWriter::create(&p, &header(r)).unwrap(), 64);
        for i in 0..5 {
            rec.push(&frame(i, r).view());
        }
        assert_eq!(rec.finish().unwrap(), 5);

        let mut src = ReplaySource::open(&p, false).unwrap();
        src.set_region(Rect::new(1, 1, 2, 2));
        for i in 0..5 {
            let f = src.next_frame(Duration::ZERO).unwrap().unwrap();
            assert_eq!(f.seq, i + 1);
            assert_eq!(f.region, Rect::new(1, 1, 2, 2));
            let want = frame(i, r);
            assert_eq!(f.px(2, 2), want.view().px(2, 2));
        }
        assert!(matches!(
            src.next_frame(Duration::ZERO),
            Err(CaptureError::SourceLost)
        ));
    }

    #[test]
    fn unfinished_clip_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.ahclip");
        let r = Rect::new(0, 0, 4, 4);
        let mut w = ClipWriter::create(&p, &header(r)).unwrap();
        w.push(&frame(0, r).view()).unwrap();
        drop(w);
        assert!(ClipReader::open(&p).is_err());
    }
}
