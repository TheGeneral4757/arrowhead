# CLAUDE.md

Guidance for Claude Code in this repo.

## Project
arrowhead: screen-reading Funky Friday (Roblox) autoplayer, 4-key ("4K") mode only, for private servers. Dev setup: game on a 2560x1440 165 Hz monitor (DP-2). Rust Cargo workspace + Tauri 2 app with a React/TS front end. Linux first (KDE Plasma 6 test target, Hyprland daily), Windows in v0.2. Successor to `~/GitHub/fnf-thing-v2` (reference code for the PipeWire, uinput, DXGI and SendInput backends).

**Status:** M0-M4 merged. M4 (timing, input, safety): tracker, planner, scheduler, uinput sink, KWin focus probe, portal hotkeys, exit/panic hooks, `cli latency` and `cli run` built; live runs work with a hand-set offset (-5.2 ms in the profile's `latency.game_bias_ms`). Readback moved to v0.2 with the HUD reader. M5 (Linux app) on branch `m5`: the live run moved to `arrowhead-runner` (shared by CLI and app), app commands + event bus, Live/Setup/Settings pages, release workflows and PKGBUILD; manual checklist on KDE/Hyprland still to do. See `docs/specs/04-timing.md`, `07-app-ui.md` (M5 notes) and `11-roadmap.md`.

## Where things are
| Need | Read |
|---|---|
| What the product is | `docs/vision/VISION.md` |
| Why a choice was made | `docs/vision/decisions.md` |
| UI look and layout | `docs/vision/design-review.html` (Layout 1, Console palette) |
| How a subsystem must work | `docs/specs/NN-*.md` (index: `00-index.md`) |
| Unverified game facts | `docs/specs/12-open-questions.md` |
| Crates and threads | `ARCHITECTURE.md` |

## Rules
- Specs are the source of truth. Change the spec in the same commit as code that departs from it.
- Treat `docs/research/ff-refs/` images as rough examples only. Never take positions, sizes, colours, shapes or HUD layout from them. Detection learns everything from the user's screen.
- Unverified game facts (judgment windows, keybinds, ms sign, accuracy weights) are config values with defaults, never bare constants.
- `arrowhead-core` stays OS-free except `runtime::platform`. Engine code talks to `FrameSource`, `KeySink`, `FocusProbe`, `Clock` traits so tests can drive it.
- Scan and scheduler threads: no allocation, locking or I/O in the hot path.
- Every press needs a release (see safety invariants S1-S5 in `docs/specs/05-input-safety.md`).
- No network calls except the update check. No telemetry.
- Conventional commits, one PR per milestone. Straight quotes and plain hyphens.
- Do not run the app or send keys against the real game without the user asking: it acts on their Roblox account.

## Releases and release notes
release-please opens the release PR; merging it tags `vX.Y.Z` and runs `release.yml`. Full flow: `docs/specs/10-release.md`.

| Part | Rule |
|---|---|
| Changelog | Only feat, fix and perf show (`release-please-config.json`). Write user-facing commit subjects. |
| Highlights | Before merging a release PR, Claude writes 1-3 plain sentences under the version heading in both the PR's notes and `CHANGELOG.md`: what changed for the player. |
| Release body | Version heading, highlights, `### Install` (from `.github/release-install.md`, added by CI), `### Features` / `### Bug Fixes`, then the footer "Made by James Thiessen with Claude Code (Claude Opus 5.5)" linking `#ai-use`. CI adds Install and footer; do not add them by hand. |
| Owner and AI | James Thiessen is the maker (LICENSE, PKGBUILD maintainer, README). Keep the AI use note in README, PKGBUILD and every release. |
| AUR | `arrowhead-bin`: CI commits the PKGBUILD bump to main and publishes once `AUR_SSH_KEY` and `AUR_EMAIL` secrets exist (not yet: AUR registration was closed on 2026-11-22). |
| README images | `assets/screenshots/{live,setup,settings}.png`, 1600 px wide. Replace with the same names. |

## Commands
| Task | Command |
|---|---|
| Tests | `cargo test --workspace` |
| Lint | `cargo clippy --workspace --all-targets -- -D warnings` |
| Dev app | `npm --prefix ui run tauri dev` |
| Release app | `npm --prefix ui run tauri build` (never `cargo build --release -p arrowhead-app` alone: it skips the UI build) |
| CLI | `cargo run -p arrowhead-cli -- <capture|record|replay|import|detect|run|bench|diag>` |
| Detect on a clip | `cargo run --release -p arrowhead-cli -- detect clip.ahclip -q --plan --debug-png out/ --every 60` (add `--profile` to start like a live run: saved layout, colours, offset, style) |
| Play (dry run, no keys) | `cargo run --release -p arrowhead-cli -- run --seconds 60` (add `--live` to send keys) |
| Focus probe / scheduler bench | `cargo run --release -p arrowhead-cli -- focus` / `-- bench` |
| Input latency test (reference only, not used as offset) | `cargo run --release -p arrowhead-cli -- latency` |
| Clip from a recording | `cargo run -p arrowhead-cli -- import ~/ah-clips/song-01.mkv -o tests/clips/x.ahclip --region x,y,w,h --start 300 --seconds 5` |

Clips in `tests/clips/` are git LFS: `git lfs pull` first.

## Linux notes
- Capture uses the xdg-desktop-portal ScreenCast picker on first run; the restore token is single-use and rewritten every run.
- `/dev/uinput` is writable via a uaccess ACL on the dev box. Hotkeys use the GlobalShortcuts portal, with the evdev + `input` group path as fallback.
- Game runs in Sober (`org.vinegarhq.Sober`). Known risk: Sober issue #2027 (16 fps while focused on KDE).
