---
created: 2026-11-21
status: draft
milestone: M1
---

# 01 Workspace, toolchain, CI

## Layout

```
arrowhead/
  Cargo.toml              workspace, shared [workspace.dependencies], resolver 3, edition 2024
  rust-toolchain.toml     channel = "stable" (1.99 at time of writing; ashpd needs >= 1.87)
  crates/
    core/      arrowhead-core
    capture/   arrowhead-capture
    input/     arrowhead-input
    app/       arrowhead-app      (Tauri 2; tauri.conf.json here)
    overlay/   arrowhead-overlay
    cli/       arrowhead-cli      (binary name: arrowhead-cli)
  ui/                     React + TS + Vite front end for the app
  tests/
    clips/                recorded clips (git LFS) + labels
  packaging/aur/          PKGBUILD for arrowhead-bin
  docs/
  .github/workflows/
```

## Dependencies (pin in `[workspace.dependencies]`)

ashpd, pipewire, windows, windows-capture, evdev, ocrs/rten, sctk, tauri were checked on crates.io 2026-11-21 ([research](../research/apis.md)). The rest are from memory: check the latest on crates.io when M1 starts. Bump deliberately, one PR per breaking bump.

| Crate | Version | Used by | Why |
|---|---|---|---|
| ashpd | 0.14 (`screencast`, `global_shortcuts`, `remote_desktop`, tokio) | capture, input | Portal ScreenCast, GlobalShortcuts. 0.14 is the zbus 6 line; stream-tuple change in portal v6 |
| pipewire | 0.10 | capture | Needs libpipewire-0.3 headers + clang |
| windows | 0.62 | capture, input, core(platform) | Win32/DXGI. Matches windows-capture |
| windows-capture | 2.0 | capture | WGC + DXGI wrapper, used for the WGC path |
| evdev | 0.13 | input | uinput virtual keyboard, evdev hotkey fallback |
| zbus | 5 or 6 (match ashpd) | input | KWin scripting focus probe |
| libc | 0.2 | core(platform) | SCHED_FIFO, affinity, clock_nanosleep |
| crossbeam-channel | 0.5 | core | Bounded channels between threads |
| parking_lot | 0.12 | core | Off-hot-path locks |
| serde, toml, serde_json | 1 / 0.9 / 1 | core, app | Config TOML, history JSONL, IPC |
| tracing, tracing-subscriber, tracing-appender | 0.1 / 0.3 / 0.2 | all | Logs, rotating files |
| rand, rand_chacha | 0.9 / 0.9 | core | Seeded RNG for the player model (deterministic tests) |
| zstd | 0.13 | capture | Clip compression |
| ocrs, rten | 0.14 / 0.28 | core (feature `ocr`) | Song titles, glyph atlas bootstrap. Off hot path |
| dirs | 6 | core | Config/data dirs |
| smithay-client-toolkit, wayland-protocols-wlr | 0.21 / 0.3 | overlay | Layer-shell |
| tiny-skia | 0.11 | overlay | Software drawing |
| tauri | 2.12 | app | Shell |
| tauri-plugin-updater | 2 | app | Signed self-update |

Not used: xcap, scap, rdev, opencv, tesseract, AVX2 intrinsics (strips are small; scalar code is fast enough, see [03](03-detection.md#performance-budget)).

## Features
| Crate | Feature | Effect |
|---|---|---|
| core | `ocr` | Pulls ocrs/rten. On in app and cli, off in unit tests |
| capture | `pipewire` (default on Linux), `dxgi`, `wgc` (default on Windows) | Backend selection |

Use `cfg(target_os)` for OS selection, features only for optional heavy deps.

## Build commands (once M1 lands)
| Task | Command |
|---|---|
| Dev app | `npm --prefix ui run tauri dev` |
| Release app | `npm --prefix ui run tauri build` (builds the UI first; never `cargo build --release -p arrowhead-app` alone, it would point at the dev server) |
| CLI | `cargo run -p arrowhead-cli -- <cmd>` |
| Tests | `cargo test --workspace` |

Linux build deps (Arch): `webkit2gtk-4.1 libsoup3 gtk3 pipewire clang`. Ubuntu CI: `libwebkit2gtk-4.1-dev libsoup-3.0-dev libgtk-3-dev libpipewire-0.3-dev clang libxkbcommon-dev libwayland-dev`.

## CI (`.github/workflows/ci.yml`)
- Trigger: push to `main`, PRs.
- Matrix: `ubuntu-24.04`, `windows-latest`. Native builds only (pipewire blocks cross-compiling).
- Steps: checkout with LFS, rust cache, `fmt --check`, `clippy -D warnings`, `test --workspace`, UI `npm ci && npm run lint && npm run build`.
- Clip regression tests run on Linux only (they do not need a display).
- Windows job builds capture/input with Windows backends; tests that need a desktop are `#[ignore]`.

## Acceptance (M1)
- Empty crates compile on both runners, CI green.
- `arrowhead-cli --version` runs.
- Tauri app opens a window with the sidebar shell from [07](07-app-ui.md).
