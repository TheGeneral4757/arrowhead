---
created: 2026-11-21
---

# arrowhead vision

## What it is
A desktop tool that plays Funky Friday's 4-key mode for you by watching the screen and pressing keys. It is built for private servers with friends who know it is running. It plays either perfectly ("Max") or like a believable person at an accuracy you choose.

## Who it is for
One user (the author) and friends, on Linux first (CachyOS, KDE Plasma 6 as the test target, Hyprland/Caelestia as the daily desktop), then Windows. Typical setup: game on one monitor, arrowhead on the other. The author plays at 2560x1440, 165 Hz. A single monitor works too, with a click-through overlay.

## What good looks like
| Quality | Meaning |
|---|---|
| Accurate | Max hits every note in the Sick window on any chart the game ships, including holds, jacks and chords, at any scroll speed. |
| Believable | Non-Max presets produce a hit-error distribution and unstable rate in a human range. They never read as uniform random jitter. |
| Zero-config | First launch walks through permissions, monitor, auto-calibration and a latency test. After that it arms itself when a song starts. |
| Safe | It never sends keys to the wrong window. It never leaves a key stuck down, even on a crash. Panic is one key away. |
| Honest | Every figure in the UI is measured. Pipeline latency, capture rate and jitter are visible. "Copy diagnostics" gives a full report. |
| Private | No telemetry. One network call: the update check, which can be turned off. |

## What changed from v2
| v2 (fnf-thing-v2) | arrowhead |
|---|---|
| Fixed pixel coordinates per note, colour match at one point | Vertical strip per lane, notes tracked across frames |
| Press when a colour appears, plus a delay | Press at the predicted crossing time, corrected by measured latency |
| Skill slider over 3 judgment tiers | Player model with 5 tiers (Sick/Good/Ok/Bad/Miss) steered to a target accuracy |
| Manual calibration | Receptors found automatically, side and scroll direction detected |
| JSON profiles | TOML config and per-display profiles |
| No tests against real footage | Synthetic frames and recorded clips run in CI |

## Out of scope
- 5K to 9K modes.
- Script injection or memory reading. Screen and keyboard only.
- Accessibility features (not a priority).
- Public matchmaking use. The product is designed for private lobbies.

## Principles
1. Accuracy first. Performance work only where it buys accuracy.
2. Measure, do not assume. Every timing constant is either measured on the machine or marked unverified.
3. The scan thread is sacred: no allocation, locks or I/O in the hot path.
4. Pure logic in `arrowhead-core`, OS code at the edges, so the whole pipeline runs in tests on recorded or synthetic frames.
5. Small, plain UI in the Hallmark "Console" look: amber for what you set, cool blue for what was measured.
