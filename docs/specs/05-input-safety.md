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
| S2 | All keys released on any exit | `Drop` for the sink; panic hook (`std::panic::set_hook`) calls `release_all` through a global handle; SIGINT/SIGTERM/SIGHUP handler (Linux) and `SetConsoleCtrlHandler` + `WM_ENDSESSION` (Windows) do the same. Destroying the uinput device also makes the kernel release its keys |
| S3 | No keys to other windows | Focus gate checked per batch |
| S4 | Panic always works | Panic path does not go through the scheduler queue: it takes the sink lock directly |
| S5 | Dry run is clean | `NullSink` is selected at engine start; switching while running restarts the engine |

## Acceptance
- Unit tests for S1, S4 with `RecordingSink`; integration test kills the CLI with SIGKILL mid-hold and checks via an evdev reader that the key is released (kernel release on device destroy). SIGKILL cannot run hooks, which is why this matters.
- Focus probe works on KDE Plasma 6 and Hyprland with Sober (manual check list in [09](09-testing.md)).
