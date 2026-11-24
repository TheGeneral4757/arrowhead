---
created: 2026-11-21
status: draft
milestone: M2-M5
---

# 08 Config, profiles, history, logs

## Locations
| What | Linux | Windows |
|---|---|---|
| Config dir | `$XDG_CONFIG_HOME/arrowhead/` (`~/.config/arrowhead/`) | `%APPDATA%\arrowhead\` |
| Data dir | `$XDG_DATA_HOME/arrowhead/` (`~/.local/share/arrowhead/`) | `%LOCALAPPDATA%\arrowhead\` |

| File | Dir | Format |
|---|---|---|
| `config.toml` | config | Global settings |
| `profiles/<name>.toml` | config | Per display setup calibration |
| `styles/<name>.toml` | config | User styles (built-in presets are compiled in and cannot be edited, only copied) |
| `screencast_token` | data | Portal restore token |
| `history.jsonl` | data | One line per song |
| `songs.json` | data | Per-song memory ([below](#songsjson)) |
| `sessions/<date>-<n>.jsonl` | data | Per-note log for one song (optional, `log.per_note = true`) |
| `glyphs/<profile>.bin` | data | Glyph atlas (binary: magic `AHGL`, version 1; [03](03-detection.md#teaching-the-atlas-bootstrap)) |
| `logs/arrowhead.<date>.log` | data | tracing, daily rotation, keep 7 |

All writes are atomic (write `*.tmp`, fsync, rename). Unknown keys are kept on save (`toml_edit`), so hand edits survive.

## `config.toml`

```toml
version = 1
profile = "default"          # active profile
style = "Max"                # active style

[keys]
set = "wasd"                 # wasd | arrows | custom
custom = ["a", "s", "w", "d"]

[hotkeys]
toggle = "F8"
panic = "F12"
nudge_up = "F9"
nudge_down = "F10"
cycle_style = "F11"

[safety]
dry_run = false
require_focus_guard = false
max_hold_ms = 15000
game_window_classes = ["org.vinegarhq.Sober", "sober", "RobloxPlayerBeta.exe"]

[capture]
linux_source = "monitor"     # monitor | window
windows_backend = "auto"     # auto | dxgi | wgc-monitor | wgc-window

[timing]
commit_ms = 8
late_limit_ms = 40
tap_hold_ms = 35
min_gap_ms = 10

[game]                       # unverified values, see specs/12
windows_ms = { sick = 45, good = 80, ok = 130, bad = 230 }
accuracy_weights = { sick = 1.0, good = 0.75, ok = 0.5, bad = 0.25, miss = 0.0 } # rough feed-forward only: real accuracy is not count-weighted (G2)
readback_sign = -1           # game shows late as negative ms (confirmed, G4)
seat = "auto"                # auto (side of the stats panel) | left | right: which score is ours

[match_friend]               # spec 06; off for Max
on = false
margin_pct = 0.0             # where to land against their score, percent of it (-3 to +3)

[ui]
overlay = "auto"             # auto (single monitor only) | on | off
minimise_while_playing = "auto"
overlay_output = ""          # output name for the overlay (e.g. "DP-2"); empty: match the capture's size and rate

[updates]
check = true

[log]
level = "info"
per_note = false
```

## `profiles/<name>.toml`

```toml
version = 1
monitor = "DP-2"             # portal/DXGI output name
stream_size = [2560, 1440]
refresh_hz = 165
scale = 1.0
side = "centre"              # centre (ours, default) | left | right
scroll = "up"
receptors = [[1702.4, 1288.1], [1846.0, 1288.3], [1989.7, 1288.0], [2133.2, 1288.2]]  # example values
radius_px = 61.5
nudge = { dx = 0, dy = 0, dpitch = 0.0, dwidth = 0.0 }

[colours]                    # learned, opponent space (C1, C2, Y); absent until calibrated
heads = [[...], [...], [...], [...]]
head_tol = [..]
trail = [..]                 # absent if not learned
trail_tol = ..
head_len = [..]              # run length of one head per lane, px

[layout]                     # full layout from the last calibration (receptors, size, run_len, run_shape, scroll, stream); lets a run skip the receptor search

[latency]
capture_ms = 6.1
input_ms = 4.8
game_bias_ms = 0.0

[hud]                        # regions found by search; absent = not readable
stats = [x, y, w, h]         # 7-row stats panel (Accuracy first)
counts = [x, y, w, h]        # 5-row judgment counts
ms_readout = [x, y, w, h]
score_left = [x, y, w, h]    # by screen side; the engine maps to self / opponent
score_right = [x, y, w, h]
accuracy = [x, y, w, h]      # unused: accuracy is the stats panel's first row
score_self = [x, y, w, h]
score_opponent = [x, y, w, h]
title = [x, y, w, h]

[detect]                     # overrides of the defaults in specs/03
```

A profile is matched at start by `(monitor, stream_size, scale)`; if none matches, Setup's Calibrate step runs and creates one. Profiles are not portable between machines or between Windows and Linux.

## `history.jsonl` line

```json
{"v":1,"started":"2026-11-21T21:42:03Z","duration_s":142.5,"song":"...","difficulty":"Hard","profile":"default","style":"Sweaty","target":96.0,
 "accuracy_hud":96.21,"counts":{"sick":612,"good":41,"ok":9,"bad":2,"miss":3},"mean_err_ms":1.2,"ur":143.0,
 "offset_ms":-11.0,"engine_missed":0,"watchdog_releases":0,"seed":123456789,
 "hit_hist":{"from_ms":-230,"bin_ms":10,"counts":[0,0,1,"..."]}}
```

`hit_hist` (optional, v0.2) holds the song's hit errors for the History page: `counts[i]` covers `[from_ms + i * bin_ms, from_ms + (i + 1) * bin_ms)`. Rows without hit errors leave it out. The app reads the file newest first and skips lines it cannot parse.

## `songs.json`
Per-song memory (`core::adapt::SongBook`), keyed by `SongBook::key(title)`: the OCR'd title lowercased, punctuation and runs of whitespace turned into single spaces. Written atomically after a play. Unknown fields are kept on save. A file that does not parse is renamed `songs.json.bad` and an empty book is used.

```json
{"version":1,"songs":{"hello world remix":{
  "name":"Hello World (Remix)","difficulty":"Hard","plays":3,"last_played":"2026-11-23T10:00:00Z",
  "offset_ms":-6.4,"lane_ms":[-1.2,0.0,0.4,0.9],"class_ms":[0.0,-1.5,0.8],
  "styles":{"casual":{"last_accuracy":87.0,"target":90.0}},
  "max_note":{"lane":[{"good":2,"ok":0,"bad":0,"miss":1},{},{},{}],"class":[{},{"good":3},{}]}}}}
```
- `offset_ms`, `lane_ms`, `class_ms`: the readback bias and the `Bias` corrections at the end of the last play with at least 20 readbacks (`MIN_READBACKS`); shorter plays keep the old values. They are the next play's start values (the profile's `latency.game_bias_ms` still applies when a song is new). `class_ms` order: Single, Chord, Jack.
- `styles.<id>`: last final accuracy and the target of the last play with that style; drives the target adjust ([06](06-humanizer.md#accuracy-controller)).
- `max_note`: non-Sick judgment counts (Good, Ok, Bad, Miss) per lane and per class from the last Max play, for the UI; the timing side is covered by `lane_ms` and `class_ms`.
- `StartValues { offset_ms, lane_ms, class_ms, target_adjust, max_note }` is what the runner reads at the start of a play.

## Diagnostics ("Copy diagnostics")
Plain text: version, OS, desktop and compositor, capture backend + stats, input backend, focus probe, hotkey backend, active profile (without file paths in the home dir), last 200 log lines at warn+, last 5 history lines. No network upload; it goes to the clipboard only.
