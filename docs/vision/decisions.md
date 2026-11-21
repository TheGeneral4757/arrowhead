# arrowhead - decisions log (Q&A rounds, 2026-11-21)

Successor to fnf-thing-v2 (fnf-turbo-rust). Full redesign, same job: screen-reading autoplayer for Funky Friday (Roblox). Used in private servers with friends.

## Identity
| Item | Decision |
|---|---|
| Name | arrowhead |
| License | MIT |
| Repo | ~/GitHub/arrowhead, public GitHub (TheGeneral4757) |
| Tone | Straight, technical |
| Visual | Clean pro tool, dark only, accent chosen via Hallmark |
| Logo | Options shown in artifact |
| v2 repo | Archive on GitHub once arrowhead exists (push d600e1a, README pointer) |

## Stack
| Item | Decision |
|---|---|
| Core | Rust, Cargo workspace (core, capture, input, app, cli) |
| GUI shell | OPEN - decide in artifact (Tauri 2 / egui-iced-Slint / localhost web UI) |
| Frontend | OPEN - depends on GUI shell |
| Layout | OPEN - compare sidebar+pages vs single dashboard in artifact |
| Windows builds | GitHub Actions CI (win + linux matrix), releases on tag |
| Linux capture | xdg-portal ScreenCast + PipeWire only |
| Windows capture | WGC, DXGI fallback |
| Input | uinput (Linux), SendInput scan codes (Windows) |
| Config | TOML in OS config dir; per-session history log |
| Logging | tracing + rotating files, "copy diagnostics" |

## Detection and timing
- Lane strip scanning (vertical strip above each receptor), classic CV, deterministic.
- Text: glyph templates for digits/judgments/ms popup; OCR only for song titles.
- Predictive timing: track note across frames, fit scroll speed, schedule press at crossing; latency auto-calibration.
- Read in-game ms/hit-error display if enabled (OCR of digits) for judgment readback.
- Accuracy features: chart-aware lookahead, judgment readback self-correction, adaptive colour thresholds, frame timing telemetry, sub-frame interpolation, input lag compensation, hold-end precision, scroll speed auto-detect.
- Gameplay: hold notes, upscroll + downscroll, side autodetect. 4K only (no 5-9K).
- Auto-calibration: detect receptors from screen, manual nudge after.
- Arming: auto-arm when receptors appear, disarm on results screen.
- Perf: capture lane region only at full monitor refresh; accuracy first.

## Humanizer (new system)
- Player-profile model (bias, spread, difficulty-scaled error for chords/jacks/streams, fatigue, focus drift) steered by a target-accuracy controller.
- UI: simple (target accuracy slider + preset picker) + advanced drawer.
- Presets are named player styles; perfect preset is named "Max".
- Friend-lobby mode: read opponent score/accuracy live, stay within a chosen margin ahead/behind.
- Dropped from v2: humanize curve, Computer Mode toggle, AVX2 path.

## Controls and safety
- Default binds: Funky Friday accepts both ASWD and arrow keys by default; configurable.
- Hotkeys: global toggle, live accuracy nudge, panic (release all), preset cycle.
- Safety: focus guard (Sober/Roblox focused only), stuck-key watchdog, release-all on exit/crash, dry-run mode.

## Displays
- Primary target: 2 monitors (game on one, arrowhead on other). Single monitor supported.
- Find game window, follow it; scale-aware coords; windowed + fullscreen; monitor picked in wizard.
- Single monitor: overlay-first (main window minimises while playing).
- Debug view: annotated lane view in-app by default; optional click-through overlay (layer-shell / Windows topmost layered).
- Live view: lane visualiser, hit-error graph, judgment counters, latency/FPS.

## Features
- Song detection (title/difficulty), per-session log.
- Offline test harness (recorded clips through detector in CI).
- Profiles: calibration per display, player style presets.
- Guided setup wizard: permissions (input group, uinput, portal), monitor pick, auto-calibrate, latency test.
- Updates: notify + one-click from GitHub releases (AUR installs defer to pacman). Only network call; can be disabled. No telemetry.

## Distribution
- GitHub Releases (Windows exe/msi, Linux tarball), AUR (arrowhead-bin, PKGBUILD in repo first, publish optional via CI action).

## Process
- Phased milestones, parallel agents within each phase. v0.1 = Linux-first MVP (4K, detection + timing + Max preset + basic UI). Windows + humanizer v0.2.
- Conventional commits, PRs per milestone, release-please.
- Testing fully automated: synthetic frames, recorded-clip regression (git LFS), humanizer statistical tests, CI on Win + Linux.
- Hallmark: decision artifact, app UI, lint hook in repo, maybe landing page.
- Docs: README, ARCHITECTURE.md, CLAUDE.md, CONTRIBUTING, plus vision docs (describe what it is and should be; later sessions refine into specs).
- Not a priority: accessibility features.

## This session
- Artifact only. Repo created after approval.

## Research deltas (2026-11-21, see research-*.md in scratchpad)
- Five judgment tiers: Sick/Good/Ok/Bad/Miss (~45/80/130/230 ms, inferred). v2 lacked Bad.
- In-game ms popup confirmed (signed readout under judgment, bottom corner of playfield).
- Accuracy formula unknown; default keybinds (ASWD + arrows) unconfirmed by sources.
- Arrow style/colours are player-customisable -> learn colours per profile.
- RISK: Sober issue #2027, 16 fps when focused on KDE Plasma 6 Wayland/CachyOS. Test first.
- GUI: Tauri 2 (2.12.x) recommended; overlay as separate process (layer-shell / layered window); browser fallback with same front end.
- CONFLICT: user picked WGC-first on Windows; research recommends DXGI-first, WGC per-window/fallback.
- CI: native runner per OS (pipewire crate blocks cross-compile).
- Self-check humanizer output with unstable rate (circleguard definition).

## Final picks (2026-11-21)
- Layout 1: sidebar + pages.
- GUI: Tauri 2 + React/TypeScript (Vite); overlay as separate small process.
- Logo: Receptor (up-arrow receptor outline with amber hit line).
- Windows capture: DXGI first, WGC per-window option + fallback.
