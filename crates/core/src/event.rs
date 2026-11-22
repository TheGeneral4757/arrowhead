//! Engine events for the app, CLI and (v0.2) overlay (spec 07, events). Times are monotonic ms
//! (`Nanos` / 1e6) so the UI can line up notes, planned and actual presses.

use serde::Serialize;
use serde::ser::{SerializeSeq, Serializer};

use crate::config::Scroll;
use crate::session::State;

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SuppressReason {
    Focus,
    Paused,
    DryRun,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Info,
    Warn,
    Error,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct Dot {
    pub id: u32,
    pub lane: u8,
    /// Stream y of the last observation.
    pub y: f32,
    /// Hold end (trail end) rather than a head.
    pub tail: bool,
}

pub const MAX_DOTS: usize = 48;

/// Fixed-size list so the scan thread can build it without allocating.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dots {
    pub n: u8,
    pub dots: [Dot; MAX_DOTS],
}

impl Default for Dots {
    fn default() -> Self {
        Self {
            n: 0,
            dots: [Dot::default(); MAX_DOTS],
        }
    }
}

impl Dots {
    pub fn push(&mut self, d: Dot) {
        if (self.n as usize) < MAX_DOTS {
            self.dots[self.n as usize] = d;
            self.n += 1;
        }
    }

    pub fn as_slice(&self) -> &[Dot] {
        &self.dots[..self.n as usize]
    }
}

impl Serialize for Dots {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut seq = s.serialize_seq(Some(self.n as usize))?;
        for d in self.as_slice() {
            seq.serialize_element(d)?;
        }
        seq.end()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct Pipeline {
    pub fps: f32,
    pub interval_p50_ms: f32,
    pub interval_p99_ms: f32,
    /// Frame age when the scan thread got it (now - pts), p50.
    pub pts_age_ms: f32,
    pub scan_us_p99: f32,
    /// Scheduler lateness (actual - planned press), p99.
    pub late_us_p99: f32,
    pub offset_ms: f32,
    /// Fitted scroll speed, px per ms (absolute); 0 before the first fit.
    pub speed_px_ms: f32,
    pub presses: u32,
    pub missed_by_engine: u32,
    pub suppressed: u32,
    pub watchdog: u32,
}

/// `None` = not readable (yet).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct Stats {
    /// Accuracy the game shows, percent.
    pub accuracy: Option<f32>,
    /// Sick, Good, Ok, Bad, Miss as the game shows them.
    pub counts: [Option<u32>; 5],
    /// From readback this song.
    pub mean_err_ms: Option<f32>,
    pub ur: Option<f32>,
    pub readbacks: u32,
    /// The style's target, percent; None for Max.
    pub target: Option<f32>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type")]
// Notes is large on purpose: boxing it would allocate on the scan thread for every frame.
#[allow(clippy::large_enum_variant)]
pub enum EngineEvent {
    State {
        state: State,
        dry_run: bool,
        reason: &'static str,
    },
    Layout {
        receptors: [[f32; 2]; 4],
        size: f32,
        scroll: Scroll,
        stream: [u32; 2],
        profile: String,
    },
    /// Tracked notes for the lane visualiser.
    Notes {
        t_ms: f64,
        dots: Dots,
    },
    Planned {
        id: u32,
        lane: u8,
        at_ms: f64,
        release_ms: f64,
    },
    Pressed {
        id: u32,
        lane: u8,
        planned_ms: f64,
        actual_ms: f64,
    },
    Released {
        id: u32,
        lane: u8,
        planned_ms: f64,
        actual_ms: f64,
    },
    Suppressed {
        id: u32,
        lane: u8,
        reason: SuppressReason,
    },
    Pipeline(Pipeline),
    /// Focus probe result: None when there is no probe.
    Focus {
        focused: Option<bool>,
    },
    Hotkeys {
        bound: Option<String>,
        error: Option<String>,
    },
    Log {
        level: Level,
        text: String,
    },
    /// One of our hits read back from the game's ms readout (spec 04, readback).
    Readback {
        /// Hit error, ms, positive = late.
        ms: f32,
        /// The count row that went up with it, when exactly one did; else from the windows.
        judgment: &'static str,
        /// Offset after this read.
        offset_ms: f32,
    },
    /// Song totals from the HUD and readback, about twice a second while playing.
    Stats(Stats),
    /// The engine stopped; `error` is set when it stopped on an error.
    Stopped {
        error: Option<String>,
    },
}

pub fn ms(t: crate::time::Nanos) -> f64 {
    t.0 as f64 / 1e6
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_shape() {
        let mut dots = Dots::default();
        dots.push(Dot {
            id: 3,
            lane: 1,
            y: 10.5,
            tail: false,
        });
        let j = serde_json::to_string(&EngineEvent::Notes { t_ms: 1.0, dots }).unwrap();
        assert_eq!(
            j,
            r#"{"type":"Notes","t_ms":1.0,"dots":[{"id":3,"lane":1,"y":10.5,"tail":false}]}"#
        );
        let j = serde_json::to_string(&EngineEvent::Pipeline(Pipeline::default())).unwrap();
        assert!(j.starts_with(r#"{"type":"Pipeline","fps":0.0"#), "{j}");
    }
}
