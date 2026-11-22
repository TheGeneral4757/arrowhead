---
created: 2026-11-21
status: draft
milestone: M5 (Live, Setup, Settings), v0.2 (Style, History, overlay)
---

# 07 App UI and overlay

Visual reference: [design-review.html](../vision/design-review.html) (Layout 1, Console palette, Receptor logo). The prototype's figures are simulated; the layout, copy and tokens are the spec.

## Shell
- `arrowhead-app`: Tauri 2. Front end in `ui/`: React + TypeScript + Vite. Plain CSS with the design tokens from the prototype `:root` block copied into `ui/src/tokens.css` (no Tailwind, no component library, no chart library: canvases drawn by hand as in the prototype).
- Dark only. Fonts: Host Grotesk (UI), Atkinson Hyperlegible Mono (numbers), bundled locally (no Google Fonts call at runtime: no network rule).
- Colour roles: amber (`--color-accent`) for things the user sets, cool blue (`--color-cool`) for measured values. Judgment colours from `--judge-*`.
- Window: 1100 x 760 default, min 900 x 600. Remembers size and position.

## Layout 1: sidebar + pages
Top bar: logo + name, state pill (Idle / Searching / Armed / Playing / Paused / Dry run), capture pill (backend + monitor), focus pill. Sidebar: pages with number keys 1-5; footer shows version, OS and active profile.

| Page | v | Content |
|---|---|---|
| Live (1) | 0.1 | Song title + difficulty, side, scroll, fitted speed. Controls: preset pill, Dry run toggle, Pause/Resume, Panic. Lane visualiser canvas (4 lanes, tracked notes, planned presses, actual presses). Accuracy + 5 judgment counters (with targets in v0.2). Hit error bar (last 40, windows shaded). Pipeline metrics with sparklines: capture fps, capture-to-press ms, frame jitter, offset applied |
| Style (2) | 0.2 | Presets, target slider, match-my-friend toggle + margin, expected distribution bar + curve, Advanced drawer with every model field from [06](06-humanizer.md). v0.1 shows only "Max" and an explanatory line |
| Setup (3) | 0.1 | Wizard: Permissions, Screen, Calibrate, Latency, Keys ([below](#setup-wizard)). Each step re-runnable |
| History (4) | 0.2 | One row per song: when, song, preset, accuracy, Sick/Good/Ok/Bad/Miss, mean error, UR. Empty state text. Click a row for its hit-error histogram |
| Settings (5) | 0.1 | Hotkeys, key set, safety toggles, capture backend, updates on/off, log level, "Copy diagnostics", "Open logs folder" |

When there is only one monitor (or the user chooses), the main window minimises on `Playing` and the overlay takes over.

## Setup wizard
| Step | Checks / actions | Pass when |
|---|---|---|
| Permissions | `/dev/uinput` writable; hotkey backend (GlobalShortcuts portal or evdev + `input` group); portal ScreenCast present; game window running | uinput and portal OK (hotkeys optional) |
| Screen | Opens the portal picker; shows the monitor chosen (name, size, refresh, scale) | A stream is running |
| Calibrate | "Start any song". Shows the snapshot with detected receptors, strips, side, scroll; drag to nudge; Re-detect button | Layout locked and saved to profile |
| Latency | Latency test window ([04](04-timing.md#latency-calibration)); shows capture, input, total offset | 50 trials done |
| Keys | Key set picker (A S W D / arrows / custom); lane test during a song | All 4 lanes confirmed or skipped |

## Tauri commands (front end -> Rust)
`get_status`, `start_engine`, `stop_engine`, `set_paused(bool)`, `panic`, `set_dry_run(bool)`, `get_config`, `set_config(patch)`, `list_profiles`, `select_profile(name)`, `run_setup_step(step)`, `calibration_snapshot() -> png bytes + layout`, `nudge_layout(dx, dy, dpitch, dwidth)`, `list_styles`, `select_style(name)`, `save_style(style)`, `history(limit, offset)`, `copy_diagnostics() -> string`, `check_update`, `install_update`.

## Events
`EngineEvent` (core) is sent on a Tauri `Channel`, batched by the bus at 30 Hz (lane visualiser at 60 Hz):

| Event | Payload |
|---|---|
| `State` | `SessionState`, reason |
| `Layout` | `LaneLayout`, profile name |
| `Notes` | tracked notes per lane (id, y, kind) for the visualiser |
| `Planned` / `Pressed` / `Released` | lane, times, track id |
| `Suppressed` | lane, reason (focus, dry run) |
| `Readback` | ms, judgment |
| `Stats` | accuracy, counts, mean error, UR, k, target |
| `Pipeline` | fps, interval p50/p99, pts age, scan us, scheduler lateness, offset |
| `Song` | title, difficulty |
| `Warning` / `Error` | code, text |

## Overlay (`arrowhead-overlay`, v0.2)
- Separate process spawned by the app; reads JSON lines on stdin (the same `EngineEvent`s, filtered), draws nothing else.
- Wayland: layer-shell surface, layer `overlay`, anchored to the game's output, keyboard interactivity none, empty input region (click-through). Draws with tiny-skia into a shm buffer.
- Windows: `WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_NOACTIVATE`.
- Content: strip boxes, receptor marks, planned press ticks, last judgment, hit error bar, state pill. Off by default on two-monitor setups.
- Spike first (M0/v0.2): does a layer-shell surface show above fullscreen Sober on KWin and Hyprland?

## M5 notes (2026-11-22)
- Commands as built: the list above minus the v0.2 ones (`list_styles`, `select_style`, `save_style`, `history`), plus `app_info`, `get_profile`, `latency_key`, `test_lane(lane)`. The contract is `ui/src/api.ts`.
- `nudge_layout` sets the profile's absolute nudge (not a delta); the runner applies it on top of the detected layout. `dpitch` spreads the receptors per lane around the centre, `dwidth` grows the receptor size. A fresh detection that moves the receptors resets it.
- Latency step: the app window is the test window. Rust emits `latency-paint` (bool) to paint it; the UI turns it white on any keydown and calls `latency_key`.
- Events are sent as batches of `EngineEvent` (core `event.rs`): `Notes` (latest only) every 16 ms, the rest every 33 ms. `Readback`, `Stats` and `Song` wait for the HUD reader (v0.2), so Live shows accuracy, judgments, hit error and the song title as empty states. `Pipeline` is 1 Hz.
- Session states come from `session::Session`: Searching while calibrating, Armed with a layout, Playing while confirmed heads exist, back to Armed after 3 s without one (no history in v0.1). Dry run is a scheduler gate, so it can be toggled while playing; without a writable uinput the app plays dry only.
- Minimise on `Playing` waits for the overlay (v0.2). Window size and position are remembered (window-state plugin).

## Acceptance
- M5: the app drives the same engine as the CLI; Live updates at 30 Hz with under 3% CPU for the webview on the dev machine; Setup completes from a clean config.
- Every number shown is measured (no placeholder figures in release builds).
