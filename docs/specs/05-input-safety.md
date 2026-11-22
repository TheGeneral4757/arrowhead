---
created: 2026-11-21
status: draft
milestone: M4 (Linux), v0.2 (Windows)
---

# 05 Input, focus guard, hotkeys, safety

## KeySink (`arrowhead-core::input`)

```rust
pub trait KeySink: Send {
    fn send_batch(&mut self, actions: &[KeyAction]) -> Result<(), InputError>; // one atomic batch
    fn release_all(&mut self) -> Result<(), InputError>;
    fn name(&self) -> &'static str;
}
```

| Backend | Crate | Notes |
|---|---|---|
| `UinputSink` (Linux) | evdev | Port v2 `input/linux.rs`. Device name "arrowhead virtual keyboard". Enable only the keys in the active key set. One `emit()` per batch (single SYN_REPORT). After creating the device, wait until it shows up in `/dev/input` (max 1 s) before first use |
| `SendInputSink` (Windows) | windows | Port v2 `input/windows.rs`. `KEYEVENTF_SCANCODE`; add `KEYEVENTF_EXTENDEDKEY` for arrow keys. One `SendInput` call per batch |
| `LibeiSink` (Linux, fallback) | ashpd RemoteDesktop + reis | Only if `/dev/uinput` is not writable. Not in v0.1; spec stub only |
| `NullSink` | - | Dry run. Records actions with timestamps, shown on Live and the overlay |
| `RecordingSink` | - | Tests. Records (time, action) against the `Clock` |

## Key sets
| Set | Left | Down | Up | Right |
|---|---|---|---|---|
| `wasd` (default) | A | S | W | D |
| `arrows` | Left | Down | Up | Right |
| `custom` | any evdev / scan code | | | |

Whether the game accepts both sets by default is unverified ([12](12-open-questions.md)). Setup's Keys step includes a check: press each lane key once during a song and confirm the receptor lights up.

## Focus guard (`FocusProbe`, focus thread, 10 Hz)

```rust
pub trait FocusProbe: Send { fn game_focused(&mut self) -> Result<bool, ProbeError>; fn name(&self) -> &'static str; }
```

| Backend | Method |
|---|---|
| Hyprland | `$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE/.socket.sock`, command `j/activewindow`; match `class` against `game_window_classes` |
| KWin (Plasma 6) | Load a generated KWin script through `org.kde.KWin /Scripting loadScript`; the script connects to `workspace.windowActivated` and calls back a DBus service arrowhead owns, sending `resourceClass` and `caption`. Use `kdotool` as reference |
| X11 / XWayland | `x11rb`: `_NET_ACTIVE_WINDOW` and `WM_CLASS`. Used when `DISPLAY` is set and no compositor backend works |
| Windows | `GetForegroundWindow` + `QueryFullProcessImageNameW` = `RobloxPlayerBeta.exe` |

- `game_window_classes` default: `["org.vinegarhq.Sober", "sober", "RobloxPlayerBeta.exe"]` (config).
- The result goes into an `AtomicBool` read by the scheduler.
- On focus lost: release all keys at once, emit `FocusLost`, state `Paused`. On regain: back to the previous state.
- If no probe works: fail open with a persistent warning pill ("Focus guard off") and a Settings toggle to require it (`safety.require_focus_guard`, default false on Linux v0.1 until both compositor probes are proven).

## Hotkeys

| Action | Default | Notes |
|---|---|---|
| Toggle (pause / resume) | F8 | |
| Panic (release all + stop engine) | F12 | Works even when paused |
| Accuracy nudge up / down | F9 / F10 | +/- 0.5% target, or +/- 1 ms offset in Max |
| Cycle preset | F11 | |

| Backend | Method |
|---|---|
| GlobalShortcuts portal (KDE, Hyprland) | ashpd `global_shortcuts`. First choice: no group membership needed. The desktop may ask the user to confirm bindings once |
| evdev watcher (Linux fallback) | Needs the `input` group. Ignore events from our own virtual device |
| Windows | `RegisterHotKey` on a message-only window |

Setup's Permissions step shows which backend is active; the `input` group hint appears only when the portal is unavailable.

## Safety invariants

| # | Rule | Mechanism |
|---|---|---|
| S1 | No key stays down longer than planned | Watchdog in the scheduler thread: every 50 ms, any lane down for more than `planned_release + 250 ms` (hard cap `safety.max_hold_ms`, default 15000) is released and logged as `WatchdogRelease` |
| S2 | All keys released on any exit | `Drop` for the sink; the panic hook and the SIGINT/SIGTERM/SIGHUP handlers (Linux) set the global `runtime::platform::EXIT` flag, and the scheduler thread checks it every loop (at most 50 ms) and releases all. `SetConsoleCtrlHandler` + `WM_ENDSESSION` on Windows in v0.2. Destroying the uinput device also makes the kernel release its keys |
| S3 | No keys to other windows | Focus gate checked per batch |
| S4 | Panic always works | Panic does not go through the scheduler queue: it sets `EXIT`, which the scheduler checks every loop; the process then exits, which destroys the uinput device. No lock on the hot path |
| S5 | Dry run is clean | `NullSink` is selected at engine start; switching while running restarts the engine |

## Acceptance
- Unit tests for S1, S4 with `RecordingSink`; integration test kills the CLI with SIGKILL mid-hold and checks via an evdev reader that the key is released (kernel release on device destroy). SIGKILL cannot run hooks, which is why this matters.
- Focus probe works on KDE Plasma 6 and Hyprland with Sober (manual check list in [09](09-testing.md)).

## M4 status (2026-11-21)
- `UinputSink` (evdev 0.13): device "arrowhead virtual keyboard" with only the 4 lane keys, one `emit` per batch, waits for the device node then 300 ms for the compositor. Not yet run against the game.
- `KwinProbe`: serves `org.arrowhead.Focus` on the session bus, loads a generated KWin script with `org.kde.kwin.Scripting.loadScript`/`start`, unloads it on drop. Checked on Plasma 6: reports the active window's `resourceClass` within 0.1 s (`cli focus`). Sober's class still to be confirmed with the game running.
- `cli run` polls the probe at 10 Hz into the scheduler's focus gate; on focus loss everything is released. No report yet counts as focused (fail open) unless `safety.require_focus_guard`.
- The `input` group is not set on the dev box, so the evdev hotkey fallback cannot read keyboards; hotkeys wait for the GlobalShortcuts portal. Until then: alt-tab away (focus gate) or Ctrl+C in the terminal; the kernel releases uinput keys when the process exits.

## M4 status update (2026-11-22)
- Hotkeys: `PortalHotkeys` (`arrowhead-input::hotkeys`) binds toggle, panic, nudge up/down and cycle preset through the GlobalShortcuts portal, registering the host app id `org.arrowhead.Arrowhead` first. Bound in the background so `cli run` does not wait for the desktop's confirm dialog. Toggle pauses (same gate as focus loss: everything released, presses suppressed); nudge moves the offset by +/- 1 ms; cycle preset waits for v0.2. Checked on the user's desktop 2026-11-22: keys work. The evdev fallback is not built (no `input` group on the dev box).
- S2/S4: `install_exit_hooks` (signals + panic hook) and the scheduler's `EXIT` check are built; Ctrl+C now stops `cli run` cleanly with all keys released. The SIGKILL integration test is not built.


## v0.2 Windows status (2026-11-22)
Built and cross-checked for `x86_64-pc-windows-msvc`, never run on Windows. Pure parts (scan codes, hotkey strings, exe matching) are unit-tested on every target in `arrowhead_input::winutil`.
- `SendInputSink` (`winsend`): scan codes, `KEYEVENTF_EXTENDEDKEY` on arrows only, one `SendInput` per batch, key-up for all lanes on `release_all` and `Drop`. A short count from `SendInput` is an error that names UIPI (an elevated game ignores a normal process).
- `WindowsProbe` (`winfocus`): `GetForegroundWindow` + `QueryFullProcessImageNameW`, file name compared with `game_window_classes` (case-insensitive, `.exe` optional). A process it cannot open (elevated) reads as not focused, so keys stop: fails safe. `cli focus` prints the foreground exe.
- `WinHotkeys` (`winhotkeys`): `RegisterHotKey` with `MOD_NOREPEAT` on a message-only window in its own thread, same five actions and `start` shape as `PortalHotkeys`. Config strings: `F1`-`F24`, letters, digits, `esc`, `space`, with `ctrl+`/`shift+`/`alt+`/`win+`. A key another program owns is reported and skipped; none bound is an error.
- S2: `install_exit_hooks` adds `SetConsoleCtrlHandler` (sets `EXIT`; for close/logoff/shutdown waits 300 ms) and a hidden top-level window that sets `EXIT` on `WM_ENDSESSION` (message-only windows do not receive it). SIGKILL-equivalent (TerminateProcess) cannot run hooks and there is no kernel-side release like uinput's: a key could stay down until the next key event. The scheduler's S1 watchdog does not help after the process is gone.
- Departure: spec said the probe matches only `RobloxPlayerBeta.exe`; it matches the whole `game_window_classes` list.
- Untested: all of it. Open: P7 (anti-cheat), P11 (elevated game).

## v0.2 status (hotkeys)
Cycle preset (F11) moves to the next style in `list_styles`, saves it as `config.toml` `style` and plays it from the next note. Nudge (F9/F10) moves the style's target by 0.5% while a style plays, else the offset by 1 ms as before.
