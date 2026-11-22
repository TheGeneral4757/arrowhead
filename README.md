<p align="center"><img src="assets/logo.svg" width="96" alt="arrowhead logo"></p>

# arrowhead

A screen-reading autoplayer for Funky Friday (Roblox), for private servers with friends. It reads the four lanes off your screen, predicts when each note crosses the receptor and presses the key on time. A player model can make it play like a person at the accuracy you pick, or perfectly with the **Max** preset.

Successor to fnf-thing-v2. Rewritten from scratch in Rust.

> **Status: v0.1 in progress.** The Linux app and CLI play from the screen (M5). See [the roadmap](docs/specs/11-roadmap.md).

> **Risk.** Automating input in Roblox breaks its Terms of Use and can get an account banned. arrowhead is for private servers with people who know. You carry the risk.

## What it does

| Area | v0.1 (Linux) | v0.2 |
|---|---|---|
| Capture | xdg-desktop-portal ScreenCast over PipeWire | + Windows DXGI, Graphics Capture fallback |
| Detection | Lane strips above each receptor, learned colours, holds, up/downscroll, side detection | + HUD reading (accuracy, opponent score) |
| Timing | Note tracking, fitted scroll speed, press at predicted crossing, latency auto-calibration | + ms-popup readback self-correction |
| Play style | Max (every note centred) | Player model + target-accuracy controller, presets, match-my-friend |
| Input | uinput virtual keyboard | + Windows SendInput scan codes |
| Safety | Focus guard, stuck-key watchdog, release-all on exit/crash, panic hotkey, dry run | same |
| UI | Tauri 2 app: Live, Setup, Settings | + Style, History, click-through overlay |

4 keys only. No telemetry. The only network call is the update check, which can be turned off.

## Install (Linux)

| Way | How |
|---|---|
| Tarball | Download `arrowhead-X.Y.Z-linux-x86_64.tar.gz` from the releases, unpack, run `./arrowhead` (the app) or `./arrowhead-cli` |
| AUR | `arrowhead-bin` (updates come from your package manager) |

Runtime deps: webkit2gtk-4.1, libsoup3, gtk3, pipewire, xdg-desktop-portal plus a backend (`xdg-desktop-portal-kde` or `xdg-desktop-portal-hyprland`). Key output needs a writable `/dev/uinput`. First run: open Setup and go through Permissions, Screen, Calibrate, Latency and Keys.

## Docs

| Doc | What |
|---|---|
| [docs/vision/VISION.md](docs/vision/VISION.md) | What arrowhead is and should be |
| [docs/vision/decisions.md](docs/vision/decisions.md) | Decision log from planning (2026-11-21) |
| [docs/vision/design-review.html](docs/vision/design-review.html) | Clickable UI prototype, stack and logo review |
| [docs/specs/](docs/specs/00-index.md) | Detailed specs, one per subsystem |
| [docs/research/](docs/research/) | Research notes: APIs, game mechanics, screenshots, prior art |
| [ARCHITECTURE.md](ARCHITECTURE.md) | Crates, threads, data flow |
| [CONTRIBUTING.md](CONTRIBUTING.md) | Workflow, commits, tests |

## Platforms

| OS | Target | Notes |
|---|---|---|
| Linux (KDE Plasma 6, Hyprland) | v0.1 | Game via Sober (`org.vinegarhq.Sober`). Needs PipeWire and a writable `/dev/uinput`. |
| Windows 10/11 | v0.2 | Built in CI. Whether Roblox's anti-cheat tolerates synthetic input is unknown. |

## License

MIT. See [LICENSE](LICENSE).
