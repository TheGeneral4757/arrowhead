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
    /// What the HUD reader found (core `HudView`): regions in stream pixels, last reads.
    Hud {
        stats: Option<[i32; 4]>,
        counts: Option<[i32; 4]>,
        ms: Option<[i32; 4]>,
        ms_read: Option<f32>,
        title: Option<[i32; 4]>,
        scores: [Option<[i32; 4]>; 2],
        score_values: [Option<i32>; 2],
        seat: Option<String>,
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
    /// HUD regions to outline, with labels: (rect in stream pixels, label).
    pub hud: Vec<([i32; 4], String)>,
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
            Msg::Hud {
                stats,
                counts,
                ms,
                ms_read,
                title,
                scores,
                score_values,
                seat,
            } => {
                let ours = seat.as_deref().map(|s| usize::from(s == "right"));
                let score = |i: usize| {
                    let who = match ours {
                        Some(o) if o == i => "you",
                        Some(_) => "opp",
                        None => "score",
                    };
                    let v = score_values[i].map_or("?".into(), |v| v.to_string());
                    format!("{who} {v}")
                };
                let hud: Vec<([i32; 4], String)> = [
                    (stats, "stats".to_string()),
                    (counts, "counts".to_string()),
                    (
                        ms,
                        format!("ms {}", ms_read.map_or("?".into(), |v| format!("{v:.1}"))),
                    ),
                    (title, "title".to_string()),
                    (scores[0], score(0)),
                    (scores[1], score(1)),
                ]
                .into_iter()
                .filter_map(|(r, l)| Some((r?, l)))
                .collect();
                let changed = hud != self.hud;
                self.hud = hud;
                changed
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

    #[test]
    fn hud_view_from_core_labels_sides() {
        use arrowhead_core::event::{EngineEvent, HudView};
        let e = EngineEvent::Hud(HudView {
            ocr: "ready",
            stats: Some([1, 2, 3, 4]),
            ms_read: Some(-12.34),
            ms: Some([5, 6, 7, 8]),
            scores: [Some([0, 9, 9, 9]), Some([9, 9, 9, 9])],
            score_values: [Some(-395), None],
            seat: Some("left"),
            ..HudView::default()
        });
        let l = serde_json::to_string(&[e]).unwrap();
        let mut d = Draw::default();
        for m in parse_batch(&l) {
            assert!(d.apply(m));
        }
        let labels: Vec<&str> = d.hud.iter().map(|h| h.1.as_str()).collect();
        assert_eq!(labels, ["stats", "ms -12.3", "you -395", "opp ?"]);
    }
}
