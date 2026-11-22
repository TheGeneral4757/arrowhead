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
- Scales with the window: the root font size runs from 16px (1280 x 800 and smaller) to 20px (2560 x 1440 and larger), whichever axis is smaller deciding; spacing, radii and control sizes are in rem so the whole UI scales with it. The lane canvas fills the window height. Content is capped at `--container` (90rem) and centred, so wider windows grow the gutters, not the grids.

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

### v0.2 status (overlay)
Built, not yet run on a real desktop: P4 in [12-open-questions.md](12-open-questions.md) stays unverified until the overlay is tried over fullscreen Sober on KWin and Hyprland.
- `arrowhead-overlay [--output NAME] [--size WxH]`: JSON array of events per stdin line, exits when stdin closes. The app passes the profile's `monitor` and `stream_size`. The output is the one named `NAME`, else the first whose current mode is `WxH`, else the first. On Linux the portal does not name the output (`monitor` is the stream id), so the size match is what picks it.
- Redraws are capped at 30 Hz on both backends: each one re-renders and copies a full-output buffer (about 15 MB at 2560x1440), which must not take CPU from the game.
- Input is a mirror of `EngineEvent` (`State`, `Layout`, `Notes` as `t_ms` only, `Planned`, `Pressed`, and `Readback {ms, judgment}`); anything else is ignored. The app sends only those, `Notes` at most every 100 ms. `Readback` is not emitted by the engine yet, so the judgment text and hit error bar stay empty until the HUD reader lands.
- Drawing is a pure `render` module (tiny-skia, unit tested without a display). Layout px are stream px: everything scales by surface width / stream width. Strip boxes run from each receptor to the edge notes come from. Text is a built-in 5x7 capitals font (no font dependency).
- Wayland (smithay-client-toolkit): layer `overlay`, anchored to all edges, exclusive zone -1, no keyboard, empty input region, shm `Argb8888`, redraw on change, after the previous frame callback and at most at 30 Hz. Integer output scale only; fractional scaling is stretched by the compositor but stays aligned.
- Windows (`cfg(windows)`): layered popup (`WS_EX_LAYERED | TRANSPARENT | TOPMOST | NOACTIVATE`), `UpdateLayeredWindow` with a premultiplied BGRA DIB, per-monitor DPI aware, redraw capped to the monitor refresh and 30 Hz. Cross-checked with `cargo clippy --target x86_64-pc-windows-msvc`; not run.
- App (`crates/app/src/overlay.rs`): spawns the binary next to the app (or `/usr/lib/arrowhead/arrowhead-overlay`) per engine start, writes through a bounded queue (a stuck overlay drops batches, never stalls the bus), kills it when the engine stops. `[ui] overlay`: `on`, `off`, or `auto` = only with one monitor. `[ui] minimise_while_playing`: `on` always, `auto` only while the overlay is up, `off` never; minimises on `Playing`, restores on Idle or Results (the Armed gap between songs keeps it down).

## v0.2 notes (Style and History pages)
- Commands added: `list_styles`, `select_style(name)`, `save_style(style)` (saves a user style and makes it active; built-in names refused), `delete_style(name)` (falls back to Max when it was active), `style_preview(style)` (the model's feed-forward gain, judgment split, spread and mean with this config's windows and weights; `StylePlayer::preview`), `history(limit, offset)` (newest first). The contract is `ui/src/api.ts`.
- Style: picking a preset saves `config.toml` `style` at once; a running engine picks it up at its next start. Any edit (target or an advanced field) makes an unsaved draft; "Save and use" stores it as `Custom (based on X)` unless renamed. Max disables the target slider and hides the advanced drawer. The expected-result panel is labelled a model estimate (accuracy is not count-weighted, G2). Match my friend is not shown until the engine reads the opponent's score.
- History: table of `history.jsonl` rows (50 per page, "Load older songs"), loading, empty and error states; picking a row shows its `hit_hist` over the judgment windows.

## Acceptance
- M5: the app drives the same engine as the CLI; Live updates at 30 Hz with under 3% CPU for the webview on the dev machine; Setup completes from a clean config.
- Every number shown is measured (no placeholder figures in release builds).
