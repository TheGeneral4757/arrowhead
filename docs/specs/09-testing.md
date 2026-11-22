---
created: 2026-11-21
status: draft
milestone: all
---

# 09 Testing

Everything that can run without a screen or keyboard runs in CI. The engine is driven through `FrameSource`, `KeySink`, `FocusProbe` and `Clock`, so tests swap all four.

## Layers
| Layer | Where | What |
|---|---|---|
| Unit | each crate | Classifier, run splitting, Theil-Sen, tracker association, scheduler invariants, glyph reader, config round-trip |
| Synthetic end-to-end | `core/tests/` | Chart -> rendered frames -> engine with `VirtualClock` + `RecordingSink` -> compare with ground truth |
| Clip regression | `tests/clips/` (LFS) | Recorded game footage -> detector + planner -> compare with labels |
| Model statistics | `core/tests/style_*.rs` | [06](06-humanizer.md#statistical-tests-09) |
| Manual | checklist below | Real game, real desktop |

## Synthetic frame generator (`core::testkit`, feature `testkit`)
Inputs: a chart (list of `{lane, time_ms, hold_ms}`), a `Scene`:

| Scene field | Range covered by tests |
|---|---|
| resolution | 1280x720, 1920x1080, 2560x1440 |
| refresh / capture rate | 60, 144, 165, 240 |
| scroll | up, down |
| speed (px per ms) | 0.4 to 3.0, plus a mid-song change |
| side | centre (large, ours) + smaller opponent group on the right, also animated; left/right for robustness |
| head shape | circle, arrow, square (random per suite) |
| colours | random per lane per suite, including low-saturation and near-background colours |
| background | black, noisy, slowly animated gradient |
| trail colour | random, including grey |
| artefacts | receptor glow on press, splash on hit, judgment popup text |
| pts jitter | 0-1.5 ms |
| dropped frames | 0-5% |

The generator also writes the ground truth (crossing time per note, hold end). Shapes and colours are random on purpose: nothing about the real skin may be baked into detection ([03](03-detection.md)).

Charts: generated patterns (singles, streams up to 20 nps, jacks down to 50 ms, 2-4 note chords, holds 100 ms-4 s, holds with taps in other lanes) and a few long mixed charts.

## Clip regression
- Clips recorded in M0 and later with `arrowhead-cli record`, region only, zstd, stored as `tests/clips/<name>.ahclip` in LFS. Keep each under 50 MB.
- Labels `<name>.labels.json`: ground truth from (a) a careful manual pass with `arrowhead-cli label` (step through frames, mark crossings) or (b) Max runs with readback, where readback ms gives the true error.
- Test: detection finds every labelled note in at least 3 frames before crossing; planned crossing within +/- 4 ms of label (frame-rate limited).

## Performance tests
`cargo bench -p arrowhead-core`: classify+runs per frame, track+plan per frame. CI reports, does not fail, except a hard cap: scan p99 < 2 ms in the bench on the CI runner.

## Manual checklist (before each release)
| # | Check | KDE Plasma 6 | Hyprland | Windows |
|---|---|---|---|---|
| 1 | Setup from clean config completes | | | |
| 2 | Portal token restores without a prompt on second launch | | | n/a |
| 3 | Auto-arm on song start, disarm on results | | | |
| 4 | Max: full Hard song, no misses, readback mean within +/- 3 ms | | | |
| 5 | Holds, jacks, chords in one song | | | |
| 6 | Upscroll song | | | |
| 7 | Focus away mid-song: keys released, resumes on return | | | |
| 8 | Panic mid-hold releases | | | |
| 9 | Kill -9 mid-hold: key released | | | |
| 10 | Overlay visible above the game, click-through (v0.2) | | | |
| 11 | Styled preset lands within 0.5% of target (v0.2) | | | |
