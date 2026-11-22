//! 3-slot frame ring between a capture thread and one consumer. The producer
//! always has a free slot; the newest frame wins and overwritten unread frames
//! count as drops. Buffers grow to the largest region seen and are then reused.

use std::cell::UnsafeCell;
use std::sync::{Condvar, Mutex};
use std::time::Duration;

use arrowhead_core::frame::{CaptureError, FrameView, Rect};
use arrowhead_core::time::Nanos;

#[derive(Default)]
pub struct Slot {
    pub data: Vec<u8>,
    pub region: Rect,
    pub stride: usize,
    pub pts: Nanos,
    pub seq: u64,
}

impl Slot {
    pub fn view(&self) -> FrameView<'_> {
        FrameView {
            seq: self.seq,
            pts: self.pts,
            region: self.region,
            stride: self.stride,
            data: &self.data,
        }
    }
}

const COPY_SAMPLES: usize = 1024;

struct State {
    latest: Option<usize>,
    reading: Option<usize>,
    seq: u64,
    drops: u64,
    region: Rect,
    closed: Option<String>,
    lost: bool,
    copy_ns: [u32; COPY_SAMPLES],
    copy_n: usize,
}

pub struct Ring {
    state: Mutex<State>,
    cv: Condvar,
    slots: [UnsafeCell<Slot>; 3],
}

// SAFETY: a slot is touched only by the producer while it is neither `latest`
// nor `reading`, or by the consumer while it is `reading`; both are decided under `state`.
unsafe impl Sync for Ring {}

#[derive(Clone, Debug, Default)]
pub struct RingStats {
    pub frames: u64,
    pub drops: u64,
    /// Producer copy time per frame, newest samples, unsorted.
    pub copy_ns: Vec<u32>,
}

impl Default for Ring {
    fn default() -> Self {
        Self::new()
    }
}

impl Ring {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(State {
                latest: None,
                reading: None,
                seq: 0,
                drops: 0,
                region: Rect::default(),
                closed: None,
                lost: false,
                copy_ns: [0; COPY_SAMPLES],
                copy_n: 0,
            }),
            cv: Condvar::new(),
            slots: Default::default(),
        }
    }

    /// Region the producer should copy. Zero size = whole frame.
    pub fn set_region(&self, r: Rect) {
        self.state.lock().unwrap().region = r;
    }

    pub fn region(&self) -> Rect {
        self.state.lock().unwrap().region
    }

    /// Producer side. `fill` gets a free slot and the requested region and
    /// returns false to discard the frame.
    pub fn produce(&self, fill: impl FnOnce(&mut Slot, Rect) -> bool, copy_ns: u32) {
        let (idx, region) = {
            let s = self.state.lock().unwrap();
            let idx = (0..3)
                .find(|i| Some(*i) != s.latest && Some(*i) != s.reading)
                .unwrap();
            (idx, s.region)
        };
        // SAFETY: idx is neither latest nor reading, so the consumer cannot see it.
        let slot = unsafe { &mut *self.slots[idx].get() };
        if !fill(slot, region) {
            return;
        }
        let mut s = self.state.lock().unwrap();
        if s.latest.is_some() {
            s.drops += 1;
        }
        s.seq += 1;
        slot.seq = s.seq;
        s.latest = Some(idx);
        let n = s.copy_n;
        s.copy_ns[n % COPY_SAMPLES] = copy_ns;
        s.copy_n += 1;
        drop(s);
        self.cv.notify_one();
    }

    /// Ends the stream; the consumer gets `SourceLost` (lost) or `Backend(msg)`.
    pub fn close(&self, msg: String, lost: bool) {
        let mut s = self.state.lock().unwrap();
        s.closed.get_or_insert(msg);
        s.lost |= lost;
        drop(s);
        self.cv.notify_all();
    }

    /// Consumer side. The returned slot stays reserved until the next call.
    pub fn acquire(&self, timeout: Duration) -> Result<Option<&Slot>, CaptureError> {
        let mut s = self.state.lock().unwrap();
        s.reading = None;
        let (mut s, _) = self
            .cv
            .wait_timeout_while(s, timeout, |s| s.latest.is_none() && s.closed.is_none())
            .unwrap();
        if let Some(msg) = &s.closed {
            return Err(if s.lost {
                CaptureError::SourceLost
            } else {
                CaptureError::Backend(msg.clone())
            });
        }
        let Some(idx) = s.latest.take() else {
            return Ok(None);
        };
        s.reading = Some(idx);
        // SAFETY: idx is now `reading`, which the producer never picks.
        Ok(Some(unsafe { &*self.slots[idx].get() }))
    }

    pub fn stats(&self) -> RingStats {
        let s = self.state.lock().unwrap();
        let n = s.copy_n.min(COPY_SAMPLES);
        RingStats {
            frames: s.seq,
            drops: s.drops,
            copy_ns: s.copy_ns[..n].to_vec(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn push(ring: &Ring, v: u8) {
        ring.produce(
            |slot, _| {
                slot.data.clear();
                slot.data.push(v);
                true
            },
            1,
        );
    }

    #[test]
    fn newest_wins_and_drops_count() {
        let ring = Ring::new();
        push(&ring, 1);
        push(&ring, 2);
        push(&ring, 3);
        let slot = ring.acquire(Duration::ZERO).unwrap().unwrap();
        assert_eq!(slot.data, [3]);
        assert_eq!(slot.seq, 3);
        assert_eq!(ring.stats().drops, 2);
        assert!(ring.acquire(Duration::ZERO).unwrap().is_none());
    }

    #[test]
    fn reading_slot_is_not_overwritten() {
        let ring = Ring::new();
        push(&ring, 1);
        let slot = ring.acquire(Duration::ZERO).unwrap().unwrap() as *const Slot;
        for v in 2..10 {
            push(&ring, v);
        }
        // SAFETY: test only; the slot is still reserved for reading.
        assert_eq!(unsafe { &*slot }.data, [1]);
    }

    #[test]
    fn close_wakes_consumer() {
        let ring = Arc::new(Ring::new());
        let r = ring.clone();
        let t = std::thread::spawn(move || r.acquire(Duration::from_secs(5)).map(|s| s.is_some()));
        std::thread::sleep(Duration::from_millis(20));
        ring.close("gone".into(), true);
        assert!(matches!(t.join().unwrap(), Err(CaptureError::SourceLost)));
    }

    #[test]
    fn threaded_frames_arrive_in_order() {
        let ring = Arc::new(Ring::new());
        let r = ring.clone();
        let t = std::thread::spawn(move || {
            for v in 0..200u8 {
                push(&r, v);
                std::thread::sleep(Duration::from_micros(200));
            }
        });
        let mut last = 0;
        while let Ok(Some(slot)) = ring.acquire(Duration::from_millis(200)) {
            assert!(slot.seq > last);
            last = slot.seq;
        }
        t.join().unwrap();
        assert_eq!(last, 200);
    }
}
