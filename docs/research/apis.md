# Research: low-latency capture + input for fnf-thing (as of 2026-11-21)

Method: crates.io API for versions/dates (verified today), GitHub/GitLab release feeds, docs.rs, Microsoft Learn, portal XML spec, web search. Items tagged **[verified]** have a fetched source; **[memory]** are from prior knowledge and not re-checked; **[unknown]** means I found no evidence either way. No authoritative capture-latency benchmarks exist publicly (searched); every latency claim below must be measured on the target box (see 3.3).

## 0. What v2 does today (read from the code)

| Area | v2 state | Comment |
|---|---|---|
| Win capture | `windows` 0.56, DXGI `DuplicateOutput`, `AcquireNextFrame(1ms)`, `CopySubresourceRegion` of only the capture box into a small STAGING texture, Map | Already the right shape (copy only the box). `windows` is 6 releases behind (0.62.2). |
| Linux capture | `ashpd` 0.13 + `pipewire` 0.10, Monitor source, CursorMode::Hidden, restore token in `screencast_token.json`, retry without token on failure, SHM-only (MemFd/MemPtr) format offer, BGRx/BGRA, CPU crop in `process` | Sound. `ashpd` 0.14 is out (zbus 6, breaking). Never offers DMA-BUF, so the compositor does a GPU to CPU copy of the whole frame per buffer. |
| Win input | `SendInput` with `KEYEVENTF_SCANCODE`, batched | Correct for DirectInput/raw-input games. |
| Linux input | `evdev` 0.13 `VirtualDevice`, one `emit()` per batch (single SYN_REPORT), evdev watcher for the hotkey (needs `input` group) | Correct. |

## 1. Linux: portal ScreenCast + PipeWire

### Crates
| Crate | Latest (crates.io) | Maintenance | Notes |
|---|---|---|---|
| ashpd | 0.14.3 (2026-11-21); 0.14.0 on 2026-11-05 | Very active (3 patch releases in 3 weeks); MSRV 1.87 | 0.14 = zbus 6 port, breaking. Source: https://github.com/bilelmoussaoui/ashpd/releases **[verified]** |
| pipewire (pipewire-rs) | 0.10.1 (2026-08-19) | Active, slower cadence, GitLab: https://gitlab.freedesktop.org/pipewire/pipewire-rs | Build needs libpipewire-0.3 dev headers + clang (bindgen). Links dynamically. |
| xcap | 0.9.10 (2026-11-09) | Active | Wraps portal+PipeWire on Wayland, WGC/GDI on Windows. One-shot screenshot oriented; not a low-latency stream API. Not recommended here. |
| scap | 0.0.8 (2025-08) | Stale | Skip. |
| wayland-client 0.31.15, sctk 0.21.1, wayland-protocols-wlr 0.3.12 | Jul-Mar 2026 | Active (smithay) | Only needed if bypassing portal (wlr-screencopy/ext-image-copy-capture). |

### Portal behaviour [verified against the portal XML: https://github.com/flatpak/xdg-desktop-portal/blob/main/data/org.freedesktop.portal.ScreenCast.xml]
- Interface version 6. `SelectSources` options: `types` (bitmask: Monitor=1, Window=2, Virtual=4), `multiple`, `cursor_mode`, `restore_token`, `persist_mode`.
- **Cursor modes**: Hidden=1 (default; cursor not in frame), Embedded=2 (painted into frame), Metadata=4 (separate SPA_META_Cursor). Hidden is correct for colour sampling.
- **Restore tokens**: single-use. Each `Start` response returns a NEW token; you must overwrite the stored one every run (v2 does). Stream `id` property is stable across restored sessions (v4+). Persist mode must be requested (1 = while app running, 2 = until revoked) or no token is issued. v2 already handles an invalid token by clearing and re-prompting.
- Stream properties carry `position`, `size`, `source_type`, `mapping_id`. Node ID semantics changed in v6 (stream tuple). Check this when upgrading ashpd.
- Token pitfalls seen in the wild: KDE bugs about restore failing (https://bugs.kde.org/show_bug.cgi?id=485697, fixed in portal-kde `screencast.cpp`) and a Jul 2026 report that Plasma 6.7.2 hands out non-UUID tokens (https://groups.google.com/g/tigervnc-users/c/cdv-YPjQvkU). Do not validate token format; treat it as an opaque string.

### Window or region capture?
| Question | KDE Plasma 6 (xdg-desktop-portal-kde) | Hyprland (xdg-desktop-portal-hyprland 1.5.0, 2026-10-28) |
|---|---|---|
| Monitor source | Yes | Yes |
| Window source (`types=Window`) | Yes (picker lists windows) [memory] | Yes, via hyprland-toplevel-export. Hyprland-only; not on other wlroots compositors [verified: https://wiki.hypr.land/Hypr-Ecosystem/xdg-desktop-portal-hyprland/] |
| Region source | No portal-level region [memory] (region is a Spectacle feature) | The XDPH picker has a region mode [memory]; not re-verified |
| Restore token | Works, with historical bugs (above) | Supported since XDPH 0.2.1; 1.3.1 fixed multi-relaunch; the picker's "Allow restore token" box is OFF by default unless `allow_token_by_default` is set [verified via search of the XDPH wiki/releases] |
| Per-window restore | [unknown] whether KDE re-finds the same window after Sober restarts (window IDs change). Bug 485697 hints it matches by window, which is fragile | [unknown] same concern |

Recommendation: capture the **Monitor** and crop in your own code (what v2 does). Window capture would make coordinates window-relative (resolution/position independent, attractive for Sober) but the restore of a window source across game restarts is the weak point; offer it as an opt-in mode and fall back to monitor. Sober on Wayland likely runs through XWayland [memory, unverified]; portal window capture of XWayland windows works on both compositors AFAIK but test it.

### DMA-BUF vs SHM, cropping, latency
- SHM (MemFd): compositor renders, then does GPU to CPU readback of the **entire** frame (4K = ~33 MB BGRx per buffer) every frame; you then CPU-crop a tiny box. Cost is on the compositor side, not yours. Reports of whole-desktop stutter while PipeWire capture runs on KDE exist (https://bugs.kde.org/show_bug.cgi?id=498303, https://obsproject.com/forum/threads/system-stuttering-when-using-pipewire-screen-capture-on-kde-plasma.172753/post-636793) **[verified as reports, not benchmarks]**.
- DMA-BUF: zero-copy GPU handle. To read a small region you must import it (EGL/Vulkan, or mmap only if the modifier is LINEAR) and copy just the box. Much more code (EGL/Vulkan via `ash`/`khronos-egl`), per-driver modifier quirks. Only worth it if measurement shows readback jitter. [memory]
- Crop at source: PipeWire has `SPA_META_VideoCrop`, but it is metadata the *producer* sets; I found **no evidence** that KWin or XDPH honour a consumer-requested crop (a Gist exists for a custom module only). Treat as not available. [unknown]
- Frame delivery: compositor streams are damage driven and capped by the negotiated max framerate. v2 offers framerate range 0..1000 with default 60. Ask for max = monitor refresh and verify actual buffer cadence via `SPA_META_Header.pts` / `pw_stream_get_time`. If the game only redraws on change this is fine; if the compositor idles you can see stalls (KDE damage-tracking bug: https://bugs.kde.org/show_bug.cgi?id=510561 **[verified]**).
- Latency: **no published numbers**. Structural floor = capture happens after compositing, so you see frame N at least one compositor frame after the game submitted it, plus PipeWire hop (sub-ms to few ms) plus SHM readback. Windows DXGI/WGC are also post-DWM, so both OS paths share that floor. Only relevant difference: SHM readback time on Linux. Measure it (section 3.3).
- Only anecdotal data: OBS PipeWire render 9-10 ms vs 2 ms on Windows for 4K (render-time metric, not capture latency): https://obsproject.com/forum/threads/render-performance-significantly-worse-on-linux.173260/post-639878

### Verdict
Keep `ashpd` + `pipewire` + SHM + monitor + Hidden cursor + restore token (matches v2). Bump ashpd to 0.14.x (check the stream-tuple change), keep the SHM path, add a frame-interval/age diagnostic. Do not adopt xcap/scap. Only go DMA-BUF if measured readback > ~2 ms or compositor stutter appears.

## 2. Windows: WGC vs DXGI Desktop Duplication

| Crate | Latest | Maintenance | Notes |
|---|---|---|---|
| windows-capture | 2.0.1 (2026-08-08); last commit 2026-11-14 | Active. 2.0 added a DXGI Desktop Duplication wrapper beside WGC [verified: docs.rs/windows-capture] | Depends on `windows` ^0.62.2. Settings: `CursorCaptureSettings`, `DrawBorderSettings {Default, WithBorder, WithoutBorder}`, `MinimumUpdateIntervalSettings`, `DirtyRegionSettings`, `SecondaryWindowSettings`, `ColorFormat` [verified]. Handler is callback based (`GraphicsCaptureApiHandler`). |
| windows | 0.62.2 (2025-10-06) | Active (Microsoft) | v2 pins 0.56. Upgrading is a mechanical but non-trivial break (HRESULT/Result and some signatures changed); do it in one commit. |
| windows-sys | 0.61.2 | Active | Raw bindings, no COM helpers. |

| Aspect | WGC (Windows.Graphics.Capture) | DXGI Desktop Duplication |
|---|---|---|
| Per-window capture | Yes (HWND item), works when occluded; also monitor | No (whole output only) |
| Latency | Frame arrives via a frame pool after DWM composes; typically comparable to DD, sometimes +1 frame [memory]; **no public head-to-head found** | Direct from DWM output; `AcquireNextFrame` returns on present |
| Min frame interval | `GraphicsCaptureSession.MinUpdateInterval` exists (TimeSpan) [verified existence: https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscapturesession.minupdateinterval]. One user saw ~50 fps on a 144 Hz window even with 0 ms (https://github.com/robmikh/Win32CaptureSample/issues/82) **[anecdote]**. Required Windows build for MinUpdateInterval: [unknown], believed Win11 only | None; delivered per desktop update |
| Yellow border | Drawn by default. `IsBorderRequired=false` needs Win11 (docs: 10.0.20348) AND consent via `GraphicsCaptureAccess.RequestAccessAsync(Borderless)`; the capability `graphicsCaptureWithoutBorder` is documented for packaged apps. If denied it silently stays on [verified: https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscapturesession.isborderrequired]. Whether unpackaged desktop apps get it without a prompt: [unknown] | No border ever |
| Cursor | Toggle (`CursorCaptureSettings`) | Cursor comes separately (pointer shape); frame has no cursor |
| Limits | Win10 1903+ (monitor capture 2004+) | Fails (ACCESS_LOST) on secure desktop/UAC, mode change, some exclusive fullscreen; one duplicator per output per process limits; must recreate on loss (v2 does) |
| Rust effort | windows-capture crate or hand-roll with `windows` (Direct3D11CaptureFramePool, free-threaded) | v2 already has it |

Recommendation: **keep DXGI DD as primary** (no border, no consent, already working, copies only the box). Add WGC (via `windows-capture` 2.0.1, which now covers both) as an opt-in per-window mode and as the fallback when DD fails (exclusive fullscreen, access lost loops, hybrid-GPU outputs). Fallback chain: DD monitor -> WGC monitor (border) -> WGC window. Do not hand-write WGC unless you need to avoid the extra crate; windows-capture is more maintained than anything I would write. Note it will force `windows` 0.62.

## 3. Input

### 3.1 Injection
| Option | Crate | Status | Notes |
|---|---|---|---|
| uinput virtual keyboard | evdev 0.13.2 (2025-09-15), 2.5M dl | Maintained, stable | Kernel-level, compositor sees it as a real keyboard (works on KDE/Hyprland/XWayland/Flatpak Sober). Needs /dev/uinput write access (uaccess ACL on this box). Create device and wait for enumeration (v2 does). ~sub-ms to inject. |
| uinput alternatives | input-linux 0.7.1 (2024-08), uinput 0.1.3 (2018) | Stale | Skip. |
| libei via RemoteDesktop portal | reis 0.7.1 (2026-07-30), ashpd `remote_desktop` | Active | No device permissions, but a permission prompt/restore token and compositor-mediated (extra hop). Good fallback when uinput is not writable. [memory on support: KDE and GNOME yes, Hyprland partial/unknown] |
| Win SendInput scancodes | windows 0.62 | n/a | v2 correct (`KEYEVENTF_SCANCODE`). Batch events in one `SendInput` call. Extended keys (arrows) need `KEYEVENTF_EXTENDEDKEY`; v2 uses a/s/w/d-type keys so fine. |
| Cross-platform wrappers | enigo 0.6.1 (2025-08), rdev 0.5.3 (2023, stale), global-hotkey 0.10.0 (2026-11-09) | enigo ok; rdev dead | Not needed; v2's direct backends are better for latency. `global-hotkey` is worth considering for the Windows pause hotkey and X11 only; it does not do Wayland globals. |

Risk note: Roblox on Windows runs Hyperion anti-cheat; synthetic `SendInput` and screen scraping may be flagged. Out of scope for this research, but it affects whether the Windows backend is viable at all. [memory]

### 3.2 Focus guard
| Platform | Method | Crates |
|---|---|---|
| Windows | `GetForegroundWindow` + `GetWindowThreadProcessId` + `QueryFullProcessImageNameW` to match RobloxPlayerBeta.exe; event-driven alternative `SetWinEventHook(EVENT_SYSTEM_FOREGROUND)` | windows |
| Hyprland | Unix socket `$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE/.socket.sock` command `j/activewindow` (JSON: class, title, pid); event socket `.socket2.sock` pushes `activewindowv2>>addr`. Hand-roll with std `UnixStream` + serde_json (a few dozen lines). `hyprland` crate 0.3.13 (2025-09) is stale-ish; `hyprland-ipc` stale | std |
| KDE Plasma 6 Wayland | KWin gives no passive "active window" DBus property [unknown/likely none]. Standard trick (what `kdotool` does): load a generated KWin script via `org.kde.KWin /Scripting loadScript`, script reads `workspace.activeWindow` and returns via `callDBus` to your service. Event-driven version: a persistent script connected to `workspace.windowActivated` calling back your DBus service. `kdotool` 0.3.0 (2026-09-09) is a Rust reference implementation (https://github.com/jinliu/kdotool); one report of `windowmove` regressing on Plasma 6.6 while other commands worked (https://discuss.kde.org/t/kdotool-windowmove-not-working-in-plasma-6-6-fedora-43/44465). Script API changed Plasma 5 to 6 (use `activeWindow` property). Crate: `zbus` | zbus |
| Generic Wayland | `wlr-foreign-toplevel-management` (Hyprland yes, KWin NO [memory]); `ext-foreign-toplevel-list` has no activated state. So not portable. | wayland-protocols-wlr |
| Sober via XWayland | If the game is an X11 client, `XGetInputFocus`/`_NET_ACTIVE_WINDOW` via `x11rb` 0.14.0 may be a cheap guard because XWayland focus goes to None when a native Wayland window is active. **Plausible but unverified** here; test on KDE and Hyprland. | x11rb |

Recommendation: per-compositor trait `FocusProbe` with Windows, Hyprland-socket, KWin-script(zbus) implementations; poll at 5-10 Hz on a separate thread (not in the scan loop) and gate injection on a cached bool. Fail open with a visible warning if the probe cannot work.

### 3.3 Measuring input-to-game latency
1. **Injection to client** (software only): small test window (winit) logging `Instant` at the injection call vs the first `KeyboardInput`/`WM_INPUT` event timestamp in the client. Linux cross-check with `libinput debug-events` / `evtest` timestamps. Windows: `QueryPerformanceCounter` around `SendInput` vs a WM_INPUT handler.
2. **Closed loop** (key to pixel): test window flips a pixel colour on key-down; the same capture pipeline detects the flip; time from emit to detection = injection + render + composite + capture. Subtract capture-only (step 3) to isolate.
3. **Capture-only**: in the test window flip a pixel at a known `Instant` (colour-by-frame-counter), measure detection delay; repeat DXGI vs PipeWire. Log PipeWire `SPA_META_Header.pts` vs `CLOCK_MONOTONIC` to see buffer age.
4. **Ground truth**: 240+ fps camera or photodiode (e.g., NVIDIA LDAT/Reflex Analyzer) on the screen; or the game's own hit-timing readout (Funky Friday shows ms offset), which is the metric that matters and is what `timing_offset_ms` already calibrates.
Report p50/p99 and jitter, not just means.

## 4. Overlays (transparent, click-through)

| Platform | Mechanism | Crates / notes |
|---|---|---|
| Wayland (KDE, Hyprland) | `wlr-layer-shell` (layer OVERLAY, anchor all, keyboard interactivity none) + empty `wl_surface` input region for click-through. Both KWin and Hyprland implement layer-shell [memory; Hyprland certain, KWin believed yes]. Normal toplevels cannot be always-on-top on Wayland (winit has no always-on-top on Wayland) | `smithay-client-toolkit` 0.21.1 (2026-07-23) + `wayland-protocols-wlr` 0.3.12; `gtk4-layer-shell` 0.9.0 (2026-11-21) for GTK4; `iced_layershell`/`layershellev` 0.19.1 (2026-07-12, waycrate) for iced; `gtk-layer-shell` 0.8.2 (GTK3, 2024-12) is what Tauri's GTK3 window would need |
| Windows | `WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_NOACTIVATE` or winit `with_transparent + set_cursor_hittest(false)`. Works over borderless-fullscreen; not over exclusive fullscreen | winit 0.30.13 (2026-10-24), windows |
| Tauri 2 | `window.set_ignore_cursor_events(true)`, transparent window. No per-region hit-testing exposed (https://dev.to/manasightgg/why-i-chose-tauri-v2-for-a-desktop-overlay-in-2026-597h **[verified via search]**). Layer-shell under Tauri/tao = hand-rolled with gtk-layer-shell on the GTK3 window before it is mapped (see tauri-plugin-widgets' Linux source for a working pattern, Background layer only). No confirmed `set_ignore_cursor_events` + layer-shell combo [unknown]. Transparent webkit2gtk windows have known GPU/compositor quirks [memory] |
| egui/eframe 0.37 | `ViewportBuilder::with_transparent/with_mouse_passthrough/with_always_on_top/with_decorations(false)` [memory]; always-on-top ineffective on Wayland | No first-class layer-shell; would be a separate sctk renderer |
| KDE click-through caveat | Discussion of KWin transparent-region input pass-through limitations: https://discuss.kde.org/t/kwin-click-trough-transparent-support/41706 [verified existence, unresolved] | use empty input region instead |

Recommendation: keep overlay as an **optional separate small process/binary or window** per OS: Wayland = sctk + layer-shell drawing a few rectangles with `tiny-skia`/raw shm (no GPU needed), Windows = layered topmost window with GDI/`tiny-skia`. It only needs to draw 4 lane boxes and text; do not drag a webview/toolkit in for it. Treat as nice-to-have: calibration can be done in the main window using a screenshot instead.

## 5. GUI shell trade-offs

| | Tauri 2 (2.12.7, wry 0.57.3, tao 0.37.3; all updated 2026-11-18) | egui/eframe 0.37 (2026-10-15) | iced 0.14.0 (2025-12-07; repo active, last commit 2026-11-19) | Slint 1.19.0 (2026-11-16) | Localhost web UI (axum + browser tab) |
|---|---|---|---|---|---|
| Binary size | Exe ~5-10 MB plus system webview [memory] | ~6-15 MB (wgpu bigger than glow) [memory] | ~8-15 MB [memory] | Few MB (software renderer) to ~10 MB [memory] | Small server exe + embedded assets |
| Linux runtime deps | webkit2gtk-4.1, libsoup3, gtk3 (v2 CLAUDE.md already lists them); heavy, distro-version sensitive, known webkit GPU/DMABUF rendering issues on some NVIDIA setups [memory] | libwayland/xkbcommon/GL via dlopen; essentially none | Same as egui class (wgpu/winit) | Same class (winit + femtovg/skia/software) | A browser (already present). No webview dep |
| Windows | WebView2 (preinstalled Win10 22H2+/11) | None | None | None | Browser |
| Real-time charts | Canvas/uPlot in webview; push downsampled batches 30-60 Hz via Tauri `Channel`; JS GC jitter fine for a monitor panel | `egui_plot` 0.38 (2026-10-19), immediate mode, trivial and fast | `canvas` widget, DIY | No chart widget; DIY with Path | uPlot/Chart.js in browser, same as Tauri |
| Overlay | Whole-window click-through only; no layer-shell support out of the box | passthrough/transparent flags; no layer-shell | `iced_layershell` 0.19.1 (third party) | Not first-class [unknown] | Impossible in-browser |
| Cross-build in GH Actions | Native matrix: ubuntu runner (apt webkit2gtk-4.1-dev, libpipewire-0.3-dev, clang) + windows-latest with `tauri-apps/tauri-action`; Linux-to-Windows cross is experimental | Pure Rust: Windows can be built from Linux with `cargo-xwin`; Linux needs native build because `pipewire` crate needs libpipewire headers | Same as egui | Same; check licence: GPLv3 / royalty-free / commercial | Same as egui |
| Fit for v2 | Already built, polished UI exists (`src-ui/` React) | Rewrite UI | Rewrite UI | Rewrite UI | Rewrite shell, keep React |

Constraint that dominates: **`pipewire` needs native Linux build deps regardless of shell**, so use native per-OS runners (ubuntu + windows) for all options. Cross-compiling is not worth it.

Recommendation: **Stay on Tauri 2** (sunk cost, UI exists, webview only runs the control panel; the scan engine is in Rust threads, so webview jitter cannot affect timing). Keep real-time graphs coarse (batched at ~30 Hz). If webkit2gtk becomes the pain (NVIDIA, distro skew, install size), the lowest-effort escape is the localhost web-UI variant (keep React, replace the Tauri shell with axum + system browser), not a toolkit rewrite. egui is the best choice only if you decide to rewrite and want in-app overlays/plots; pair it with `egui_plot`. Skip iced (slow release cadence) and Slint (no chart/overlay story, licence).

## 6. Template matching and OCR

| Crate | Latest | Status | Use |
|---|---|---|---|
| (no crate) fixed-font glyph matching | n/a | n/a | **Best for digits/short words in a fixed game font**: threshold, split by column projection / connected components, compare each glyph against a reference atlas by SAD or NCC. ~10-100 us, zero deps. Capture a glyph atlas once in calibration. |
| imageproc | 0.27.0 (2026-06-02) | Active | `template_matching::match_template` (SSE/CCORR/NCC) for locating icons; CPU, slow on big frames but fine on small crops |
| fast_image_resize | 6.2.0 (2026-11-20) | Active | SIMD resize/rescale before matching or OCR |
| opencv | 0.102.0 (2026-11-08) | Active | `matchTemplate` is fast and robust, but drags in a full OpenCV system dependency; overkill |
| template-matching | 0.2.0 (2023-07) | Stale | wgpu based; skip |
| ocrs + rten | ocrs 0.14.0, rten 0.28.0 (2026-11-08) | Active, pure Rust, no system deps (https://github.com/robertknight/ocrs) | General text detection+recognition, models downloaded separately; tens to hundreds of ms per image, trained on documents, weak on tiny stylised game glyphs [memory] |
| oar-ocr | 0.11.0 (2026-11-18) | Active | PaddleOCR via ONNX Runtime (`ort`); better on scene text, brings ONNX Runtime |
| ocr-rs 2.4.1 (2026-08), paddle-ocr-rs 0.6.1 (2025-08) | | ok / stale | PaddleOCR via MNN / ort |
| tesseract-rs | 0.4.0 (2026-07-31) | Maintained | Builds Tesseract (C++), needs `eng.traineddata` |
| tesseract-plumbing 0.11.1 (2025-02), leptess 0.14.0 (2023-02), rusty-tesseract 1.1.10 (2024-03) | | stale | avoid |

Recommendation: template/glyph matching in-house for score digits and judgement words (deterministic, microseconds, runs in the scan thread); `ocrs` only as an off-hot-path fallback for unknown text (menus, song titles). Tesseract is a poor fit: slow init, needs upscaling for small glyphs.

## Uncertainties to resolve with a spike
1. Does KDE and XDPH honour window-source restore tokens for XWayland Sober across game restarts? (1)
2. KWin layer-shell overlay above fullscreen/XWayland game; Tauri `set_ignore_cursor_events` under Wayland. (4)
3. WGC `MinUpdateInterval` minimum build, borderless consent for unpackaged apps, 144 Hz cadence. (2)
4. PipeWire SHM readback cost and cadence at your monitor's refresh rate on Plasma 6 and Hyprland; DMA-BUF only if bad. (1)
5. XWayland `XGetInputFocus` as a focus guard. (3.2)
6. Roblox Hyperion on Windows vs synthetic input/capture (viability, not an API question).
