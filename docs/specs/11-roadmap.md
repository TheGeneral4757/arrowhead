---
created: 2026-11-21
status: draft
---

# 11 Roadmap

Phased milestones. Within a milestone, the listed tracks can run as parallel agents on separate task branches, merged into the milestone branch. Each milestone ends with its acceptance checks from the linked specs.

## M0: game facts and spikes (before code)
Goal: replace research guesses with measurements. Needs one private-server session and the user at the keyboard.

| Track | Output |
|---|---|
| Sober frame rate | Is Sober issue #2027 (16 fps while focused on KDE Plasma 6) present on this box? Also on Hyprland? Workaround if any |
| Record | 5+ songs recorded as full-monitor video (OBS or `gpu-screen-recorder`) at native refresh: downscroll, upscroll, holds, jacks, both sides, one skin (the user only uses one). Converted to `.ahclip` once M2 exists |
| Game facts | Answer every row of [12](12-open-questions.md): windows, ms readout sign, default keys, HUD elements and where they are, accuracy formula (from several results screens), scroll speed settings |
| Overlay spike | Layer-shell surface above fullscreen Sober on KWin and Hyprland |
| Focus spike | KWin script focus probe and Hyprland socket against Sober |

## M1: workspace and CI
Spec [01](01-workspace.md). Tracks: Cargo workspace + empty crates; CI workflow; Tauri shell + `ui/` with tokens and the sidebar; CLI skeleton.

## M2: capture, clips, config
Specs [02](02-capture.md), [08](08-config.md). Tracks: PipeWire backend (port from v2); `.ahclip` reader/writer + `ReplaySource`; config/profile/history types with atomic writes; `cli capture`, `cli record`, `cli replay`.

## M3: detection and calibration
Spec [03](03-detection.md), [09](09-testing.md). Tracks: synthetic frame generator; strip classifier + runs; receptor finder + side/scroll detection; colour learning; session state machine; `cli detect`.

## M4: timing, input, safety (Max plays)
Specs [04](04-timing.md), [05](05-input-safety.md). Tracks: tracker + scroll fit + planner; scheduler + clocks; uinput sink + safety hooks; focus probes; hotkeys; latency test; `cli run`. Exit: Max clears a Hard song on Linux from the CLI. Readback moved to v0.2 (needs the HUD reader).

## M5 = v0.1.0: Linux app
Specs [07](07-app-ui.md), [10](10-release.md). Tracks: Live page; Setup wizard; Settings; event bus; release workflow + Linux tarball + PKGBUILD. Exit: manual checklist rows 1-9 on KDE and Hyprland.

Status (2026-11-22, branch `m5`): all tracks built (shared `arrowhead-runner`, event bus, Tauri commands, Live/Setup/Settings, release-please, release.yml, PKGBUILD). Not yet done: the manual checklist on KDE and Hyprland, the 3% webview CPU check, repo secrets and the first tagged release.

## v0.2.0: Windows, player model, history, overlay
Tracks: DXGI + WGC capture; SendInput + Windows focus/hotkeys; player model + controller + presets ([06](06-humanizer.md)); HUD reader (glyph atlas, accuracy, scores, readback); Style and History pages; match my friend; overlay process; MSI build.

## v0.3.0 and later (unordered)
AUR publish automation; libei input fallback; DMA-BUF capture if measurements need it; Hallmark lint hook; landing page.
