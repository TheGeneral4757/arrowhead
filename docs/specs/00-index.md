---
created: 2026-11-21
status: draft
---

# Specs index

Specs turn the [vision](../vision/VISION.md) and [decisions](../vision/decisions.md) into buildable detail. They are the source of truth: code that disagrees with a spec is a bug in one of them, fixed in the same PR.

| # | Spec | Milestone |
|---|---|---|
| 01 | [Workspace, toolchain, CI](01-workspace.md) | M1 |
| 02 | [Capture](02-capture.md) | M2 |
| 03 | [Detection and calibration](03-detection.md) | M3 |
| 04 | [Timing and scheduling](04-timing.md) | M4 |
| 05 | [Input, focus guard, hotkeys, safety](05-input-safety.md) | M4 |
| 06 | [Player model and accuracy controller](06-humanizer.md) | v0.2 |
| 07 | [App UI and overlay](07-app-ui.md) | M5 / v0.2 |
| 08 | [Config, profiles, history, logs](08-config.md) | M2-M5 |
| 09 | [Testing](09-testing.md) | all |
| 10 | [Release and updates](10-release.md) | M5 |
| 11 | [Roadmap](11-roadmap.md) | - |
| 12 | [Open questions (game facts to verify)](12-open-questions.md) | M0 |

## Words

| Term | Meaning |
|---|---|
| Lane | One of four columns: Left, Down, Up, Right (index 0-3, in that order left to right) |
| Receptor | The fixed target shape a note must reach. One per lane, per player |
| Side | Which set of four receptors is ours: `Left` or `Right` |
| Scroll | `Down` (notes move down to receptors at the bottom) or `Up` |
| Strip | Vertical band of pixels above (downscroll) or below (upscroll) one receptor that notes travel through |
| Head | The note body that must be pressed |
| Trail | The hold-note body following a head |
| Crossing | The instant a head's centre reaches the receptor centre line. The ideal press time |
| Hit error | Press time minus crossing time, in ms. Negative = early |
| Judgment | Sick / Good / Ok / Bad / Miss |
| UR | Unstable rate: standard deviation of hit error in ms, times 10 (osu! definition) |
| Profile | Calibration for one display setup (monitor, resolution, side, colours, latency) |
| Style | Player model parameters plus a target accuracy. Presets are named styles |

## Shared types (owned by `arrowhead-core`)

Names other specs rely on. Field lists are minimums.

| Type | Shape |
|---|---|
| `Nanos(u64)` | Monotonic time, ns. Linux `CLOCK_MONOTONIC`, Windows QPC converted to ns |
| `Lane` | `enum { Left, Down, Up, Right }` |
| `Rect` | `{ x: i32, y: i32, w: u32, h: u32 }`, physical pixels of the captured stream |
| `FrameView<'a>` | `{ seq: u64, pts: Nanos, region: Rect, stride: usize, data: &'a [u8] }`, BGRx |
| `LaneLayout` | `{ side, scroll, receptors: [Point; 4], radius: f32, pitch: f32, strips: [Rect; 4] }` |
| `NoteObs` | `{ lane, y: f32, kind: Head | TrailStart | TrailEnd, pts: Nanos }` |
| `Track` | `{ id: u32, lane, obs: SmallVec<NoteObs>, predicted_cross: Nanos, hold_end: Option<Nanos> }` |
| `PressPlan` | `{ track_id, lane, press_at: Nanos, release_at: Nanos, committed: bool }` |
| `KeyAction` | `{ lane, down: bool }` |
| `Judgment` | `enum { Sick, Good, Ok, Bad, Miss }` |
| `SessionState` | `enum { Idle, Searching, Armed, Playing, Results, Paused }` |
| `EngineEvent` | See [07](07-app-ui.md#events) |

## Conventions
- Units in names: `_ms`, `_px`, `_hz`. Config values in ms; internal values in `Nanos`.
- "Unverified" in a spec means the value comes from community sources and is tracked in [12](12-open-questions.md). It must be a config value with a default, never a bare constant.
