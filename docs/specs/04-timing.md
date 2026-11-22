---
created: 2026-11-21
status: draft
milestone: M4
---

# 04 Timing and scheduling

Turns note observations into exact press and release times, and sends them on time.

## Clock
`trait Clock { fn now(&self) -> Nanos; fn sleep_until(&self, t: Nanos); }`
- `RealClock`: Linux `CLOCK_MONOTONIC` + `clock_nanosleep(TIMER_ABSTIME)` to `t - 300 us`, then spin on `now()`. Windows QPC + high-resolution waitable timer (`CREATE_WAITABLE_TIMER_HIGH_RESOLUTION`) then spin.
- `VirtualClock` for tests: `sleep_until` advances time instantly; replay sources and the scheduler share it.

## Tracking (`core::track`)
- One tracker per lane, heads and trail ends tracked separately. Associate each new `NoteObs` with the existing track whose predicted y at this frame's pts is nearest, within `min(0.35 * head_len, 15 ms of travel)` (at least 3 px) once the scroll velocity is known: the 15 ms cap keeps the next note of a 50 ms jack from being taken for this one. Before a velocity exists, a one-observation track may move up to `1.5 * head_len` toward the receptor per frame (4 px/ms at 60 Hz is 67 px).
- A track is confirmed after 3 observations moving toward the receptor; once the velocity is known, also at least 15 ms of history and a rate within 30% of the scroll speed (a lit receptor or the end of a hold drifting into it is not a note).
- Notes in one lane never overtake each other, so association is order-preserving: match sorted observations to sorted predictions (no Hungarian needed).
- Tracks end when the head passes the receptor by more than one radius, 20 ms after their predicted crossing (after that a lit receptor sits exactly where the head was and would keep the track alive), or after 120 ms unseen. A confirmed head predicted between two head observations of its lane less than `2.5 * head_len` apart is hidden under overlapping heads, not unseen.

## Scroll velocity
Scroll speed is shared across lanes and constant within a song unless the game changes it.
- Velocity `v` (px per ms, signed) is fitted with Theil-Sen over within-track (pts, y) pairs at least 10 ms apart, from all confirmed tracks in the last 500 ms; at least 6 pairs. Observations inside the receptor zone are left out (glow and splashes).
- Per-track intercept `b` = mean of `y - v t` over its last 16 observations with the lowest and highest quarter dropped (pts jitter is roughly uniform, where this beats the median). The crossing time solves `b + v t = receptor_y`.
- Speed change detection: if the median residual of the newest observations exceeds 2 px and the 150 ms fit differs from the 500 ms fit by more than 5%, use the 150 ms fit and count `scroll_changes`. (Without the 5% test, real footage's few px of noise switched on 261 of 1980 frames.)
- Sub-frame interpolation falls out of this: the crossing is computed from the fit, not from the frame where the head touched the receptor.

## Planning (`core::plan`)
For each confirmed track:
1. `cross = predicted crossing time`.
2. `target = cross + latency_offset + style_error` where:
   - `latency_offset` (negative = send earlier) cancels the measured pipeline delay, see below.
   - `style_error` comes from the player model ([06](06-humanizer.md)); 0 for Max.
3. `press_at = target`. Re-planned every frame until `press_at - now < commit_ms + 2 * frame_interval` (commit_ms default 8 ms; frame interval smoothed), then committed and handed to the scheduler. The two frames make sure the next frame (or the one after a dropped frame) cannot arrive after the commit point; with only `commit_ms`, 60 Hz presses were up to a frame late. Committed presses are not changed.
4. Late discovery: a track confirmed after its `press_at` has passed is pressed immediately if `now - cross < late_limit_ms` (default 40), else skipped (counted as `missed_by_engine`).
5. Release:
   - Tap: `release_at = press_at + tap_hold_ms` (default 35 ms, config; the game needs a press it can see for one game frame).
   - Hold: `release_at = hold_end_cross + latency_offset + style_release_error`. `hold_end_cross` comes from the `TrailEnd` track (nearest one behind the head and before the next head), refitted like heads. Releases keep being re-planned until within the commit window.
   - Hold whose end is not visible yet: at the press the lane's trail runs off the strip (`TrailCut`) and no head follows; the release is set to `max_hold` and re-planned each frame. It stays down while a trail is still attached to the receptor (`TrailStart`/`TrailCut` seen in the last 50 ms) and moves to the real end once its `TrailEnd` track appears; if the trail disappears, it is released.
   - Jack: if the next note in the lane has `press_at` within `tap_hold_ms + min_gap_ms` (default 10), the release moves to `next.press_at - min_gap_ms`. If that leaves under 8 ms of hold, the press is still sent and the gap is kept at `min_gap_ms`.
6. Chords: plans whose `press_at` is within 1 ms are merged into one batch.
7. One note seen as two tracks (a track broke up): a second head in the same lane whose crossing is within 25 ms of a committed one is the same note and is not pressed again.

## Scheduler (`core::sched`, scheduler thread)
- Queue: fixed-capacity min-heap of `KeyAction` by time (capacity 256), filled by the scan thread through a bounded SPSC channel.
- Loop: peek earliest, `clock.sleep_until(t)`, pop everything due within 0.5 ms, check the focus gate, call `KeySink::send_batch`, record actual send time, emit `EngineEvent::Pressed`.
- Invariants (also asserted in tests):
  - Never a press for a lane that is already down; never a release for a lane that is up.
  - Every press has a scheduled release at enqueue time.
  - If the focus gate is closed at send time: drop presses, still send releases, emit `Suppressed`.

## Latency calibration
Total offset = `-(capture_delay + detect_delay + input_delay) + game_bias`. When the frame pts is the presentation time (PipeWire, trusted), crossings are already predicted on the display clock, so `capture_delay` and `detect_delay` only shorten the warning time and are not subtracted; they are when pts is the receive time. `cli run` uses only `game_bias` from the profile; `--offset-ms` overrides it.

> [!warning] Measured input delay does not predict the offset (2026-11-22)
> `cli latency` measured 19.2 ms (key to the terminal flash on screen). Subtracting it made every press early; a hand-set -5.2 ms played best, about the same as 0. The game hides most of its own render delay, so `input_delay` is kept for reference only and the offset is `game_bias` (hand-set now, readback in v0.2).

| Part | How it is measured | When |
|---|---|---|
| capture_delay | `now - pts` at frame receipt (pts age p50) | Live, continuously |
| detect_delay | Scan time for the frame | Live |
| input_delay | Latency test: a test window turns white on key down; arrowhead sends a key, captures the screen through the same capture path, and measures send to seen (frame pts). Subtract the capture part (zero when pts is the presentation time). 50 trials, use p50. CLI: `cli latency`, where the terminal itself is the test window | Setup step "Latency" (app), `cli latency` |
| game_bias | Readback, below. Covers everything not measured above (game frame timing, audio sync, unknown offsets) | During play |

The offset is shown on Live > Pipeline and stored per profile.

## Readback
When the game's ms readout is readable ([03](03-detection.md#hud-reading-corehud-hud-thread-10-20-hz)):
- Each new readout is matched to the most recent committed press whose send time is 30-400 ms before the readout appeared, same lane if the judgment position tells us the lane, else by time only. Unmatched reads are dropped.
- Sign convention: the game shows late hits as negative ms, so `readback_sign = -1` (confirmed, G4).
- With readback normalised so positive = late: `game_bias -= 0.05 * (readback_ms - intended_style_error_ms)` per matched read, clamped to +/- 60 ms, starting from the profile's stored value. In Max mode this keeps the mean hit error at 0; in styled modes it keeps the intended distribution centred where the model put it.
- Implemented as `core::hud::Readback` (pure, no threads). The hud thread calls `on_blank()` when the ms region holds no text and `on_read(frame_pts, raw_ms, lane, presses)` for each tick with a valid read; `presses` is the recent committed-press ring as `SentPress { sent, lane, intended_err_ms }` (intended error positive = late, Max style 0). It returns `Matched { press, hit_error_ms, intended_err_ms, bias_ms }` for a new readout with a press 30-400 ms older (the youngest such press wins), else `None`. A read equal to the previous one is the same readout unless the region went blank in between. The runner applies `bias_ms` as the new `game_bias` and logs `hit_error_ms`. Use the frame's pts, not the time the hud thread got to it.
- Without readback, `game_bias` stays at its stored value; Setup offers a manual nudge (ms slider) and a guided "tap test" using results-screen judgments.

## Telemetry shown in the UI
Per frame: scan time. Per press: planned vs actual send time (scheduler lateness p99). Per readback: hit error. Rolling: mean hit error, UR, scroll speed, offset.

## Acceptance (M4)
- Synthetic replay with `VirtualClock`, Max style: every press within +/- 2 ms of ground-truth crossing for scroll speeds 1.0-4.0 at 60, 144 and 240 Hz capture, with 1 ms pts jitter and 2% dropped frames.
- Scheduler lateness p99 under 0.3 ms on the dev machine (real clock, idle system).
- Holds released within +/- 5 ms of trail end; jacks at 60 ms spacing never merge.
- Real game, M4 exit: a full Hard song with Max on Linux and no misses. The readback mean check (+/- 3 ms) moves to v0.2 with the HUD reader.

## M4 status (2026-11-21)
Built: `core::track`, `core::plan`, `core::sched`, `core::engine` (detect -> track -> plan on the scan thread), `RealClock::sleep_until` (absolute `clock_nanosleep` to t - 300 us, then spin) and `VirtualClock`, `cli run` (dry run unless `--live`), `cli bench`, `cli detect --plan`.

Synthetic acceptance (`core/src/engine.rs` tests): 12 suites, 60/144/240 Hz x 1-4 px/ms, both directions, 1 ms pts jitter, 2% dropped frames, random shapes/colours/backgrounds, glow and popups. Every note pressed exactly once, every press released. Worst press error 2.2 ms at 60 Hz (limit 2.5 ms there: a press is committed two frames ahead), within 2 ms at 144 and 240 Hz; notes whose heads overlap a neighbour (placed from a run end) within 3 ms; holds released within 2.1 ms of their end. Middle notes of overlapping 3+ jacks are reported separately (see spec 03 known limits); 4-6 per slow suite.

Scheduler lateness (`cli bench`, SCHED_FIFO via the `realtime` group): p99 under 1 us of the planned time on the dev machine (limit 0.3 ms).

Real footage (`cli detect --plan`, 12 s of song-01): scroll 1.94 px/ms, no speed changes, 56 presses planned (7 holds), none missed by the engine. 69 moving note tracks were seen; the 13 not pressed vanished more than ~140 ms before their crossing because the user hit them early in the recording, which cannot happen when arrowhead plays.

Update 2026-11-22: `cli latency` (terminal as the test window, result saved as the profile's `latency.input_ms`, reference only), hotkeys, panic hook and signal handlers (spec 05) are built. Readback moves to v0.2 with the HUD reader ([11](11-roadmap.md)). Run on the user's machine 2026-11-22: latency test, dry run, live runs and hotkeys all work; best offset is a hand-set -5.2 ms (`latency.game_bias_ms`).

## v0.2 Windows status (2026-11-22)
- `runtime::platform` on Windows: `monotonic_ns` = QPC in ns (`qpc_to_ns` is overflow-safe and unit-tested); `qpc_ticks_to_ns` converts DXGI's `LastPresentTime` onto the same clock. `sleep_abs_ns` = per-thread `CREATE_WAITABLE_TIMER_HIGH_RESOLUTION` timer that stops 700 us before the target (the timer can wake ~0.5 ms late), then `RealClock` spins. `make_realtime` = `THREAD_PRIORITY_HIGHEST` on the calling thread (scan, scheduler and the capture thread call it).
- Untested: `cli bench` on Windows (the p99 < 0.3 ms target is unmeasured; the 700 us margin is a guess), timer availability below Windows 10 1803 (falls back to `thread::sleep`).

## v0.2 status (readback)
Built (`crates/runner/src/hud.rs`, the hud thread): side copies of the HUD rects from the capture (`AuxTap`, about 20 Hz; full frames at about 5 Hz until the stats, counts and our ms readout are found). Each scheduler press goes to the hud thread with the error the style meant for it (`Planner::intended_err`). A readout is matched by `hud::Readback` (30-400 ms, sign -1) and moves the offset, which the scan thread applies through a channel; reads with a hit error beyond the Bad window + 20 ms are taken as misreads and ignored. The offset is saved to the profile's `latency.game_bias_ms` at the end of each song with readbacks, so the next run starts from it. Unverified until the user plays: our own ms readout has not been seen on a screenshot yet (see spec 03, G14).
