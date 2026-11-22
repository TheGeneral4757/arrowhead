//! Scheduler (spec 04) with the safety invariants of spec 05: every press has a release,
//! never press a lane that is down, focus gate on presses only, watchdog.

use serde::Serialize;

use crate::input::{KeyAction, KeySink};
use crate::plan::Cmd;
use crate::time::Nanos;

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub enum SchedEvent {
    Pressed {
        id: u32,
        lane: u8,
        planned: Nanos,
        actual: Nanos,
    },
    Released {
        id: u32,
        lane: u8,
        planned: Nanos,
        actual: Nanos,
    },
    /// Press dropped: focus gate closed.
    Suppressed {
        id: u32,
        lane: u8,
    },
    WatchdogRelease {
        lane: u8,
    },
    SinkError,
}

#[derive(Clone, Copy, Debug)]
struct Entry {
    at: Nanos,
    id: u32,
    action: KeyAction,
}

pub const QUEUE_CAP: usize = 256;
/// Everything due within this of the earliest entry goes in one batch.
const BATCH_NS: u64 = 500_000;
const WATCHDOG_GRACE_NS: u64 = 250_000_000;

pub struct Scheduler {
    queue: Vec<Entry>,
    /// Per lane: (press id, press time, planned release) while down.
    down: [Option<(u32, Nanos, Nanos)>; 4],
    batch: Vec<KeyAction>,
    due: Vec<Entry>,
    pub max_hold: Nanos,
    pub dropped: u32,
}

impl Scheduler {
    pub fn new(max_hold_ms: f64) -> Self {
        Self {
            queue: Vec::with_capacity(QUEUE_CAP),
            down: [None; 4],
            batch: Vec::with_capacity(8),
            due: Vec::with_capacity(QUEUE_CAP),
            max_hold: Nanos::from_ms(max_hold_ms),
            dropped: 0,
        }
    }

    pub fn submit(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Press {
                id,
                lane,
                at,
                release,
            } => {
                if self.queue.len() + 2 > QUEUE_CAP {
                    self.dropped += 1;
                    return;
                }
                let release = release.max(at.add_ms(1.0));
                self.queue.push(Entry {
                    at,
                    id,
                    action: KeyAction::Press(lane),
                });
                self.queue.push(Entry {
                    at: release,
                    id,
                    action: KeyAction::Release(lane),
                });
            }
            Cmd::MoveRelease { id, release } => {
                let press = self
                    .queue
                    .iter()
                    .find(|e| e.id == id && matches!(e.action, KeyAction::Press(_)))
                    .map(|e| e.at);
                let down_at = self.down.iter().flatten().find(|d| d.0 == id).map(|d| d.1);
                let Some(floor) = press.or(down_at) else {
                    return;
                };
                for e in self
                    .queue
                    .iter_mut()
                    .filter(|e| e.id == id && matches!(e.action, KeyAction::Release(_)))
                {
                    e.at = release.max(floor.add_ms(1.0));
                }
                for d in self.down.iter_mut().flatten().filter(|d| d.0 == id) {
                    d.2 = release;
                }
            }
        }
    }

    pub fn next_due(&self) -> Option<Nanos> {
        self.queue.iter().map(|e| e.at).min()
    }

    pub fn is_down(&self, lane: u8) -> bool {
        self.down[lane as usize].is_some()
    }

    /// Sends everything due by `now + 0.5 ms` in one batch. `focus` gates presses only.
    pub fn fire_due(
        &mut self,
        now: Nanos,
        focus: bool,
        sink: &mut dyn KeySink,
        events: &mut Vec<SchedEvent>,
    ) {
        let limit = Nanos(now.0 + BATCH_NS);
        self.due.clear();
        let mut i = 0;
        while i < self.queue.len() {
            if self.queue[i].at <= limit {
                self.due.push(self.queue.swap_remove(i));
            } else {
                i += 1;
            }
        }
        if self.due.is_empty() {
            return;
        }
        // Releases before presses at equal times, so a jack's release lands first.
        self.due
            .sort_by_key(|e| (e.at, matches!(e.action, KeyAction::Press(_))));
        self.batch.clear();
        let mut pressed: [Option<(u32, Nanos)>; 4] = [None; 4];
        let mut released: [Option<(u32, Nanos)>; 4] = [None; 4];
        for k in 0..self.due.len() {
            let e = self.due[k];
            let lane = e.action.lane() as usize;
            match e.action {
                KeyAction::Release(_) => {
                    if self.down[lane].is_some_and(|d| d.0 == e.id) {
                        self.down[lane] = None;
                        self.batch.push(e.action);
                        released[lane] = Some((e.id, e.at));
                    }
                }
                KeyAction::Press(_) => {
                    if !focus {
                        // Drop the press and its release.
                        self.queue.retain(|q| q.id != e.id);
                        events.push(SchedEvent::Suppressed {
                            id: e.id,
                            lane: lane as u8,
                        });
                        continue;
                    }
                    if let Some(d) = self.down[lane] {
                        // Never press a lane that is down: release it first (shortened hold).
                        self.queue.retain(|q| {
                            !(q.id == d.0 && matches!(q.action, KeyAction::Release(_)))
                        });
                        self.batch.push(KeyAction::Release(lane as u8));
                        released[lane] = Some((d.0, d.2));
                        self.flush(sink, events);
                    }
                    let rel = self
                        .queue
                        .iter()
                        .find(|q| q.id == e.id)
                        .map_or(e.at, |q| q.at);
                    self.down[lane] = Some((e.id, e.at, rel));
                    self.batch.push(e.action);
                    pressed[lane] = Some((e.id, e.at));
                }
            }
        }
        self.flush(sink, events);
        // `now` is read right after the clock's sleep, so it is the send time.
        let actual = now;
        for (lane, p) in released.iter().enumerate() {
            if let Some((id, planned)) = *p {
                events.push(SchedEvent::Released {
                    id,
                    lane: lane as u8,
                    planned,
                    actual,
                });
            }
        }
        for (lane, p) in pressed.iter().enumerate() {
            if let Some((id, planned)) = *p {
                events.push(SchedEvent::Pressed {
                    id,
                    lane: lane as u8,
                    planned,
                    actual,
                });
            }
        }
    }

    fn flush(&mut self, sink: &mut dyn KeySink, events: &mut Vec<SchedEvent>) {
        if !self.batch.is_empty() && sink.send_batch(&self.batch).is_err() {
            events.push(SchedEvent::SinkError);
        }
        self.batch.clear();
    }

    /// S1: release any lane held past its planned release + 250 ms, or past `max_hold`.
    pub fn watchdog(&mut self, now: Nanos, sink: &mut dyn KeySink, events: &mut Vec<SchedEvent>) {
        for lane in 0..4 {
            let Some((id, at, rel)) = self.down[lane] else {
                continue;
            };
            if now.0 > rel.0 + WATCHDOG_GRACE_NS || now.0 > at.0 + self.max_hold.0 {
                self.down[lane] = None;
                self.queue.retain(|q| q.id != id);
                self.batch.push(KeyAction::Release(lane as u8));
                events.push(SchedEvent::WatchdogRelease { lane: lane as u8 });
            }
        }
        self.flush(sink, events);
    }

    /// Panic / stop / focus lost: everything up, queue cleared.
    pub fn release_all(&mut self, sink: &mut dyn KeySink) {
        self.queue.clear();
        self.down = [None; 4];
        let _ = sink.release_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::RecordingSink;
    use crate::time::{Clock, VirtualClock};
    use std::sync::Arc;

    fn ms(v: f64) -> Nanos {
        Nanos::from_ms(v)
    }

    fn run(
        s: &mut Scheduler,
        clock: &Arc<VirtualClock>,
        sink: &mut RecordingSink,
        until: f64,
        focus: bool,
    ) -> Vec<SchedEvent> {
        let mut ev = Vec::new();
        while let Some(t) = s.next_due() {
            if t > ms(until) {
                break;
            }
            clock.sleep_until(t);
            s.fire_due(clock.now(), focus, sink, &mut ev);
        }
        ev
    }

    #[test]
    fn every_press_released_and_jacks_split() {
        let clock = Arc::new(VirtualClock::default());
        let mut sink = RecordingSink::new(clock.clone());
        let mut s = Scheduler::new(15000.0);
        s.submit(Cmd::Press {
            id: 1,
            lane: 0,
            at: ms(100.0),
            release: ms(135.0),
        });
        // Overlapping second press in the same lane: first must be released first.
        s.submit(Cmd::Press {
            id: 2,
            lane: 0,
            at: ms(120.0),
            release: ms(155.0),
        });
        s.submit(Cmd::Press {
            id: 3,
            lane: 2,
            at: ms(120.2),
            release: ms(150.0),
        });
        run(&mut s, &clock, &mut sink, 1000.0, true);
        let acts: Vec<_> = sink.log.iter().map(|x| x.1).collect();
        assert_eq!(
            acts,
            vec![
                KeyAction::Press(0),
                KeyAction::Release(0),
                KeyAction::Press(0),
                KeyAction::Press(2),
                KeyAction::Release(2),
                KeyAction::Release(0)
            ]
        );
        assert!((0..4).all(|l| !s.is_down(l)));
    }

    #[test]
    fn focus_gate_drops_presses_keeps_releases() {
        let clock = Arc::new(VirtualClock::default());
        let mut sink = RecordingSink::new(clock.clone());
        let mut s = Scheduler::new(15000.0);
        s.submit(Cmd::Press {
            id: 1,
            lane: 1,
            at: ms(10.0),
            release: ms(50.0),
        });
        run(&mut s, &clock, &mut sink, 20.0, true);
        s.submit(Cmd::Press {
            id: 2,
            lane: 2,
            at: ms(30.0),
            release: ms(60.0),
        });
        let ev = run(&mut s, &clock, &mut sink, 1000.0, false);
        assert!(ev.contains(&SchedEvent::Suppressed { id: 2, lane: 2 }));
        let acts: Vec<_> = sink.log.iter().map(|x| x.1).collect();
        assert_eq!(acts, vec![KeyAction::Press(1), KeyAction::Release(1)]);
    }

    #[test]
    fn watchdog_releases_stuck_lane() {
        let clock = Arc::new(VirtualClock::default());
        let mut sink = RecordingSink::new(clock.clone());
        let mut s = Scheduler::new(1000.0);
        s.submit(Cmd::Press {
            id: 1,
            lane: 3,
            at: ms(10.0),
            release: ms(5000.0),
        });
        run(&mut s, &clock, &mut sink, 20.0, true);
        let mut ev = Vec::new();
        s.watchdog(ms(500.0), &mut sink, &mut ev);
        assert!(ev.is_empty());
        s.watchdog(ms(1011.0), &mut sink, &mut ev);
        assert_eq!(ev, vec![SchedEvent::WatchdogRelease { lane: 3 }]);
        assert!(!s.is_down(3) && s.next_due().is_none());
    }

    #[test]
    fn move_release_extends_hold() {
        let clock = Arc::new(VirtualClock::default());
        let mut sink = RecordingSink::new(clock.clone());
        let mut s = Scheduler::new(15000.0);
        s.submit(Cmd::Press {
            id: 7,
            lane: 0,
            at: ms(10.0),
            release: ms(45.0),
        });
        run(&mut s, &clock, &mut sink, 20.0, true);
        s.submit(Cmd::MoveRelease {
            id: 7,
            release: ms(400.0),
        });
        run(&mut s, &clock, &mut sink, 1000.0, true);
        assert_eq!(sink.log[1], (ms(400.0), KeyAction::Release(0)));
    }
}
