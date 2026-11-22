//! What the overlay knows: a small mirror of the `EngineEvent`s it cares about, and the draw
//! state they update. `Msg` is deserialised from the JSON lines the app writes (core's
//! `EngineEvent` is serialise-only: its `reason` is a `&'static str`).

use std::collections::VecDeque;

use arrowhead_core::config::Scroll;
use arrowhead_core::session::State;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum Msg {
    State {
        state: State,
        dry_run: bool,
    },
    Layout {
        receptors: [[f32; 2]; 4],
        size: f32,
        scroll: Scroll,
        stream: [u32; 2],
    },
    /// The app sends `t_ms` only (no dots).
    Notes {
        t_ms: f64,
    },
    Planned {
        id: u32,
        lane: u8,
        at_ms: f64,
    },
    Pressed {
        id: u32,
    },
    /// Not emitted by the engine until the HUD reader lands (spec 07, v0.2): signed ms error.
    Readback {
        ms: f32,
        judgment: String,
    },
    /// Everything else (Pipeline, Log, ...): ignored.
    #[serde(other)]
    Other,
}

pub struct Layout {
    pub receptors: [[f32; 2]; 4],
    pub size: f32,
    pub scroll: Scroll,
    pub stream: [u32; 2],
}

pub const MAX_ERRORS: usize = 40;
const MAX_PLANNED: usize = 16;
/// A planned press this long behind the engine clock is stale (its Pressed never came).
const STALE_MS: f64 = 250.0;

#[derive(Default)]
pub struct Draw {
    pub state: Option<(State, bool)>,
    pub layout: Option<Layout>,
    /// (id, lane, planned ms) of presses not yet made.
    pub planned: Vec<(u32, u8, f64)>,
    pub judgment: Option<String>,
    pub errors: VecDeque<f32>,
}

impl Draw {
    /// Applies a message; true when what is on screen changes.
    pub fn apply(&mut self, m: Msg) -> bool {
        match m {
            Msg::State { state, dry_run } => {
                let new = Some((state, dry_run));
                let changed = self.state != new;
                self.state = new;
                if matches!(state, State::Idle | State::Searching) {
                    self.planned.clear();
                    self.judgment = None;
                    self.errors.clear();
                }
                changed
            }
            Msg::Layout {
                receptors,
                size,
                scroll,
                stream,
            } => {
                self.layout = Some(Layout {
                    receptors,
                    size,
                    scroll,
                    stream,
                });
                true
            }
            Msg::Notes { t_ms } => {
                let n = self.planned.len();
                self.planned.retain(|p| p.2 > t_ms - STALE_MS);
                self.planned.len() != n
            }
            Msg::Planned { id, lane, at_ms } => {
                if self.planned.len() == MAX_PLANNED {
                    self.planned.remove(0);
                }
                self.planned.push((id, lane, at_ms));
                true
            }
            Msg::Pressed { id } => {
                let n = self.planned.len();
                self.planned.retain(|p| p.0 != id);
                self.planned.len() != n
            }
            Msg::Readback { ms, judgment } => {
                if self.errors.len() == MAX_ERRORS {
                    self.errors.pop_front();
                }
                self.errors.push_back(ms);
                self.judgment = Some(judgment);
                true
            }
            Msg::Other => false,
        }
    }
}

/// One stdin line; blank or unparseable lines are skipped.
pub fn parse_batch(line: &str) -> Vec<Msg> {
    serde_json::from_str::<Vec<Msg>>(line).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_parses_and_ignores_unknown() {
        let l = r#"[{"type":"State","state":"Playing","dry_run":false,"reason":"x"},{"type":"Pipeline","fps":1.0},{"type":"Planned","id":1,"lane":2,"at_ms":5.0,"release_ms":9.0},{"type":"Notes","t_ms":1.0}]"#;
        let mut d = Draw::default();
        let msgs = parse_batch(l);
        assert_eq!(msgs.len(), 4);
        let changed: Vec<bool> = msgs.into_iter().map(|m| d.apply(m)).collect();
        assert_eq!(changed, [true, false, true, false]);
        assert_eq!(d.planned, [(1, 2, 5.0)]);
        assert!(parse_batch("garbage").is_empty());
    }

    #[test]
    fn planned_clears_on_press_and_when_stale() {
        let mut d = Draw::default();
        d.apply(Msg::Planned {
            id: 1,
            lane: 0,
            at_ms: 100.0,
        });
        d.apply(Msg::Planned {
            id: 2,
            lane: 1,
            at_ms: 120.0,
        });
        assert!(d.apply(Msg::Pressed { id: 1 }));
        assert!(!d.apply(Msg::Pressed { id: 1 }));
        assert!(d.apply(Msg::Notes { t_ms: 500.0 }));
        assert!(d.planned.is_empty());
    }
}
