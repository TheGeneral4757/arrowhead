<p align="center"><img src="assets/logo.svg" width="96" alt="arrowhead logo"></p>

<h1 align="center">arrowhead</h1>

<p align="center">A screen-reading Funky Friday autoplayer for Linux (and Windows), for private servers with friends.</p>

<p align="center">
  <a href="https://github.com/TheGeneral4757/arrowhead/releases/latest"><img src="https://img.shields.io/github/v/release/TheGeneral4757/arrowhead?color=f5b731" alt="Latest release"></a>
  <a href="https://github.com/TheGeneral4757/arrowhead/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/TheGeneral4757/arrowhead/ci.yml?branch=main&label=CI" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/github/license/TheGeneral4757/arrowhead" alt="MIT license"></a>
  <img src="https://img.shields.io/badge/platform-Linux%20%7C%20Windows-informational" alt="Linux and Windows">
</p>

<p align="center"><img src="assets/screenshots/live.png" alt="The Live page" width="900"></p>

arrowhead reads the four lanes off your screen, predicts when each note crosses the receptor and presses the key on time. It reads the game's own HUD to tune its timing, can play like a person at the accuracy you pick (or keep pace with a friend's score), and keeps a history of every song. 4 keys only. Rust + Tauri, by James Thiessen. Successor to fnf-thing-v2.

**New here?** The [user guide](docs/GUIDE.md) covers install, setup, every page and setting, how it works and troubleshooting.

> [!WARNING]
> Automating input in Roblox breaks its Terms of Use and can get an account banned. arrowhead is for private servers with people who know. You carry the risk.

## Features

- **Reads the screen, nothing else.** PipeWire screen capture through xdg-desktop-portal. No game memory, no injection.
- **Learns your setup.** Lane colours, scroll direction, side and scroll speed are detected from your screen, not hard-coded.
- **On time.** Notes are tracked across frames and pressed at the predicted crossing.
- **Tunes itself.** Reads the game's ms popups, stats panel and results screen (OCR, all on your machine) and corrects its timing from them, at any song rate.
- **Plays like a person, if you want.** Style presets with a target accuracy, your own styles, or match a friend's score. Max presses every note centred.
- **History and overlay.** Every song with the game's exact numbers and a hit-error histogram; an optional click-through overlay that keeps clear of the lanes and HUD.
- **Safe by default.** Focus guard, stuck-key watchdog, release-all on exit or crash, a panic hotkey and a dry-run mode.
- **App and CLI.** A desktop app (Live, Setup, Settings) and `arrowhead-cli` for capture, replay and diagnostics.
- **Private.** No telemetry. The only network call is the update check, which you can turn off.

| Setup | Settings |
|---|---|
| ![Setup](assets/screenshots/setup.png) | ![Settings](assets/screenshots/settings.png) |

## Install

Downloads are on the [latest release](https://github.com/TheGeneral4757/arrowhead/releases/latest).

**AppImage (any distro, updates in-app):**

```sh
chmod +x arrowhead-0.2.0-linux-x86_64.AppImage   # the version you downloaded
./arrowhead-0.2.0-linux-x86_64.AppImage
```

Needs `libfuse2` (Arch: `fuse2`) and a 2024 or newer distro (glibc 2.39: Ubuntu 24.04, Fedora 40, Debian 13, Arch). It brings its own WebKit.

**Arch (AUR):** `yay -S arrowhead-bin` (coming soon). Updates come from your package manager.

**Tarball (any distro):** grab `arrowhead-X.Y.Z-linux-x86_64.tar.gz` and `SHA256SUMS`, then:

```sh
sha256sum -c --ignore-missing SHA256SUMS
tar xzf arrowhead-*-linux-x86_64.tar.gz
cd arrowhead-*-linux-x86_64 && ./arrowhead
```

Needs webkit2gtk-4.1, libsoup3 and gtk3. The app tells you when a new version is out.

**Windows 10/11:** the `.msi` (installs, updates in-app) or the portable `.zip`. Built in CI, not yet tested on real hardware.

Every Linux install needs PipeWire and xdg-desktop-portal with your desktop's backend (`xdg-desktop-portal-kde` or `xdg-desktop-portal-hyprland`).

## Setup after installing

Once per machine, then once per monitor. Full detail in the [guide](docs/GUIDE.md#2-one-time-system-setup-linux).

**1. Let arrowhead press keys (Linux).** It types through a virtual keyboard, `/dev/uinput`. If `test -w /dev/uinput` fails, add a udev rule and log out and back in:

```sh
echo 'KERNEL=="uinput", SUBSYSTEM=="misc", TAG+="uaccess", OPTIONS+="static_node=uinput"' | sudo tee /etc/udev/rules.d/60-arrowhead-uinput.rules
echo uinput | sudo tee /etc/modules-load.d/uinput.conf
sudo modprobe uinput && sudo udevadm control --reload && sudo udevadm trigger
```

Without it the app still runs, but only in dry run.

**2. Get the game ready.** Run Roblox in [Sober](https://sober.vinegarhq.org/), join Funky Friday on a private server, 4-key mode. Note the four keys you play with in the game's keybind settings.

**3. Run the Setup page**, with the game on screen:

| Step | What to do |
|---|---|
| Permissions | Key output and screen capture must show OK (hotkeys are optional) |
| Screen | Pick the **whole monitor** the game is on. Asked once; later starts reuse it |
| Calibrate | Start any song. Check the boxes sit on your four receptors; drag to nudge or press Re-detect |
| Latency | A short key test (reference only: timing tunes itself from the game) |
| Keys | Pick WASD, arrows or custom to match your game keybinds, then test each lane in a song |

**4. Play.** On **Live**, press **Start** (from the song menu or mid-song), then play songs as usual. Turn on **Dry run** to watch without pressing. Pick a play style on the **Style** page; results land in **History**.

**Hotkeys:** F8 pause, F12 panic (release all and stop), F9 / F10 nudge the offset (Max) or the target (styles). Keys are only pressed while the game window has focus (on KDE; where focus cannot be checked, as on Hyprland, it plays and warns unless Settings requires the focus guard).

**If something is off:** see [troubleshooting](docs/GUIDE.md#10-troubleshooting), or Settings, "Copy diagnostics" for a bug report.

## Platforms

| OS | Status | Notes |
|---|---|---|
| Linux (KDE Plasma 6, Hyprland) | Since v0.1 | Game via Sober. Needs PipeWire and a writable `/dev/uinput`. |
| Windows 10/11 | v0.2, untested | Built in CI, not yet run on real hardware. Whether Roblox's anti-cheat tolerates synthetic input is unknown. |

<details>
<summary><b>What came in v0.1 and v0.2</b></summary>

| Area | v0.1 (Linux) | v0.2 |
|---|---|---|
| Capture | xdg-desktop-portal ScreenCast over PipeWire | + Windows DXGI, Graphics Capture fallback |
| Detection | Lane strips above each receptor, learned colours, holds, up/downscroll, side detection | + HUD reading (accuracy, opponent score) |
| Timing | Note tracking, fitted scroll speed, press at predicted crossing, latency auto-calibration | + ms-popup readback self-correction |
| Play style | Max (every note centred) | Player model + target-accuracy controller, presets, match-my-friend |
| Input | uinput virtual keyboard | + Windows SendInput scan codes |
| Safety | Focus guard, stuck-key watchdog, release-all on exit/crash, panic hotkey, dry run | same |
| UI | Tauri 2 app: Live, Setup, Settings | + Style, History, click-through overlay |

Full plan: [docs/specs/11-roadmap.md](docs/specs/11-roadmap.md).
</details>

<details>
<summary><b>Docs</b></summary>

| Doc | What |
|---|---|
| [docs/GUIDE.md](docs/GUIDE.md) | User guide: install, setup, pages, settings, how it works, troubleshooting |
| [docs/vision/VISION.md](docs/vision/VISION.md) | What arrowhead is and should be |
| [docs/vision/decisions.md](docs/vision/decisions.md) | Decision log from planning (2026-11-21) |
| [docs/vision/design-review.html](docs/vision/design-review.html) | Clickable UI prototype, stack and logo review |
| [docs/specs/](docs/specs/00-index.md) | Detailed specs, one per subsystem |
| [docs/research/](docs/research/) | Research notes: APIs, game mechanics, screenshots, prior art |
| [ARCHITECTURE.md](ARCHITECTURE.md) | Crates, threads, data flow |
| [CONTRIBUTING.md](CONTRIBUTING.md) | Workflow, commits, tests |
</details>

## AI use

arrowhead is made by **James Thiessen**. The entire codebase (code, specs, docs, UI and CI) was generated with AI, using Claude Code with Anthropic's Claude Opus 5.5, directed and tested by James. Commits carry a `Co-Authored-By: Claude` trailer. Review it with that in mind.

## License

MIT, Copyright (c) 2026 James Thiessen. See [LICENSE](LICENSE).
