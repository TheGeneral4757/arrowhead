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
- One tracker per lane. Associate each new `NoteObs` with the existing track whose predicted y at this frame's pts is nearest, within `max(6 px, 0.35 * head_diameter)`. Unmatched observations start tentative tracks; a track is confirmed after 3 observations with consistent direction.
- Notes in one lane never overtake each other, so association is order-preserving: match sorted observations to sorted predictions (no Hungarian needed).
- Tracks end when the head passes the receptor by more than one radius, or disappears (hit by us, or the game removed it).

## Scroll velocity
Scroll speed is shared across lanes and constant within a song unless the game changes it.
- Velocity `v` (px per ms, signed) is fitted with Theil-Sen over (pts, y) pairs from all confirmed tracks in the last 500 ms; at least 6 pairs.
- Per-track position at time t: `y(t) = y_last + v * (t - pts_last)`; the crossing time solves `y(t) = receptor_y`.
- Speed change detection: if the median residual of the last 10 observations exceeds 2 px, refit using only the last 150 ms and log `ScrollChanged`.
- Sub-frame interpolation falls out of this: the crossing is computed from the fit, not from the frame where the head touched the receptor.

## Planning (`core::plan`)
For each confirmed track:
1. `cross = predicted crossing time`.
2. `target = cross + latency_offset + style_error` where:
   - `latency_offset` (negative = send earlier) cancels the measured pipeline delay, see below.
   - `style_error` comes from the player model ([06](06-humanizer.md)); 0 for Max.
3. `press_at = target`. Re-planned every frame until `press_at - now < commit_ms` (default 8 ms), then committed and handed to the scheduler. Committed plans are not changed.
4. Late discovery: a track confirmed after its `press_at` has passed is pressed immediately if `now - cross < late_limit_ms` (default 40), else skipped (counted as `missed_by_engine`).
5. Release:
   - Tap: `release_at = press_at + tap_hold_ms` (default 35 ms, config; the game needs a press it can see for one game frame).
   - Hold: `release_at = hold_end_cross + latency_offset + style_release_error`. `hold_end_cross` comes from the `TrailEnd` track, refitted like heads.
   - Jack: if the next note in the lane has `press_at` within `tap_hold_ms + min_gap_ms` (default 10), the release moves to `next.press_at - min_gap_ms`. If that leaves under 8 ms of hold, the press is still sent and the gap is kept at `min_gap_ms`.
6. Chords: plans whose `press_at` is within 1 ms are merged into one batch.

## Scheduler (`core::sched`, scheduler thread)
- Queue: fixed-capacity min-heap of `KeyAction` by time (capacity 256), filled by the scan thread through a bounded SPSC channel.
- Loop: peek earliest, `clock.sleep_until(t)`, pop everything due within 0.5 ms, check the focus gate, call `KeySink::send_batch`, record actual send time, emit `EngineEvent::Pressed`.
- Invariants (also asserted in tests):
  - Never a press for a lane that is already down; never a release for a lane that is up.
  - Every press has a scheduled release at enqueue time.
  - If the focus gate is closed at send time: drop presses, still send releases, emit `Suppressed`.

## Latency calibration
Total offset = `-(capture_delay + detect_delay + input_delay) + game_bias`.

| Part | How it is measured | When |
|---|---|---|
| capture_delay | `now - pts` at frame receipt (pts age p50) | Live, continuously |
| detect_delay | Scan time for the frame | Live |
| input_delay | Setup latency test: the app opens a test window that changes colour on key down; arrowhead sends a key, captures the window through the same capture path, and measures send to seen. Subtract the capture part. 50 trials, use p50 | Setup step "Latency" |
| game_bias | Readback, below. Covers everything not measured above (game frame timing, audio sync, unknown offsets) | During play |

The offset is shown on Live > Pipeline and stored per profile.

## Readback
When the game's ms readout is readable ([03](03-detection.md#hud-reading-corehud-hud-thread-10-20-hz)):
- Each new readout is matched to the most recent committed press whose send time is 30-400 ms before the readout appeared, same lane if the judgment position tells us the lane, else by time only. Unmatched reads are dropped.
- Sign convention of the readout is unverified: set `readback.sign` during M0 by pressing deliberately late in a test song.
- With readback normalised so positive = late: `game_bias -= 0.05 * (readback_ms - intended_style_error_ms)` per matched read, clamped to +/- 60 ms, starting from the profile's stored value. In Max mode this keeps the mean hit error at 0; in styled modes it keeps the intended distribution centred where the model put it.
- Without readback, `game_bias` stays at its stored value; Setup offers a manual nudge (ms slider) and a guided "tap test" using results-screen judgments.

## Telemetry shown in the UI
Per frame: scan time. Per press: planned vs actual send time (scheduler lateness p99). Per readback: hit error. Rolling: mean hit error, UR, scroll speed, offset.

## Acceptance (M4)
- Synthetic replay with `VirtualClock`, Max style: every press within +/- 2 ms of ground-truth crossing for scroll speeds 1.0-4.0 at 60, 144 and 240 Hz capture, with 1 ms pts jitter and 2% dropped frames.
- Scheduler lateness p99 under 0.3 ms on the dev machine (real clock, idle system).
- Holds released within +/- 5 ms of trail end; jacks at 60 ms spacing never merge.
- Real game, M4 exit: a full Hard song with Max on Linux, readback mean within +/- 3 ms and no misses.
