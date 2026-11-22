---
created: 2026-11-21
status: draft
milestone: M2 (Linux), v0.2 (Windows)
---

# 02 Capture

Gets pixels of the lane region off the screen as fast as the monitor refreshes, with an accurate timestamp per frame.

## Interface (`arrowhead-core::frame`)

```rust
pub trait FrameSource: Send {
    /// Blocks up to `timeout`. Ok(None) on timeout. Frame borrows the ring slot until dropped.
    fn next_frame(&mut self, timeout: Duration) -> Result<Option<FrameView<'_>>, CaptureError>;
    /// Region to deliver, in stream pixels. Takes effect on the next frame.
    fn set_region(&mut self, region: Rect);
    /// Full stream size and refresh, for calibration and diagnostics.
    fn info(&self) -> SourceInfo; // { width, height, refresh_hz, backend: &'static str, monitor: String, scale: f32 }
    /// Grab one full-size frame (calibration, not hot path). May be slower.
    fn snapshot(&mut self) -> Result<OwnedFrame, CaptureError>;
}
```

- Pixel format out: BGRx, 4 bytes per pixel. Callers read (r, g, b) through `FrameView::px(x, y)`.
- `pts` is the presentation time of the frame (when it was put on screen), converted to `Nanos` on the same clock as `Clock::now()`. If the backend gives no pts, use receive time and set `SourceInfo.pts_is_receive_time = true`.
- `CaptureError` variants: `PermissionDenied`, `SourceLost` (monitor unplugged, mode change, ACCESS_LOST), `Backend(String)`. `SourceLost` makes the runtime rebuild the source and drop to `Searching`.

## Linux: portal ScreenCast + PipeWire (`capture::pipewire`)

Base it on v2 `src-tauri/src/scanner/pipewire.rs` (727 lines, known working). Changes from v2:

| Item | Spec |
|---|---|
| ashpd | 0.14. Re-check node id handling against portal v6 stream tuples |
| Source type | Monitor (`types = 1`). Window capture is an opt-in setting (`capture.linux.source = "window"`), off by default: restore of window sources across Sober restarts is unverified |
| Cursor | `Hidden` |
| Persist | `persist_mode = 2` (until revoked). Every `Start` returns a new token: overwrite the stored one each run. Treat the token as an opaque string |
| Token file | `<data_dir>/screencast_token` (plain text). On restore failure: delete, re-prompt once |
| Formats | Offer BGRx and BGRA, SHM (MemFd, MemPtr). Do not offer DMA-BUF in v0.1 |
| Frame rate | Request `max = refresh_hz` of the chosen output (from stream `size`/portal info; fallback 240). Min 0 |
| Crop | CPU copy of `region` out of the mapped buffer into the ring slot. The compositor will not crop for us |
| Timestamp | `SPA_META_Header.pts` (ns, CLOCK_MONOTONIC on PipeWire). Validate it is within 100 ms of now; if not, fall back to receive time and log once |
| Ring | 3 slots, preallocated to the largest region seen. Producer overwrites the oldest unread slot (newest frame wins; count drops) |
| Thread | PipeWire main loop on its own thread; `process` callback does the crop and signals the consumer (futex/condvar) |

### Diagnostics exposed
`frames_per_sec`, `interval_p50/p99_ms`, `pts_age_p50/p99_ms` (receive time minus pts), `drops`, `copy_us_p99`. Shown on Live > Pipeline and in "copy diagnostics".

### Escalation (only if measured bad)
If `copy_us_p99 > 2000` or compositor stutter is seen while capturing: add a DMA-BUF path (EGL import, copy the region on GPU). Not before.

## Windows (v0.2)

| Order | Backend | When |
|---|---|---|
| 1 | DXGI Desktop Duplication | Default. Port v2 `scanner/dxgi.rs`: `AcquireNextFrame`, `CopySubresourceRegion` of the region only into a staging texture, `Map`. Use `DXGI_OUTDUPL_FRAME_INFO.LastPresentTime` (QPC) as pts |
| 2 | WGC monitor (windows-capture) | DXGI fails 3 times in 5 s (ACCESS_LOST loop, exclusive fullscreen, hybrid GPU) |
| 3 | WGC window | Opt-in setting, or when 2 fails. Coordinates become window-relative |

WGC draws a yellow border unless borderless capture is granted; request it, and say in Setup if it is still shown. Set `MinUpdateInterval` to 0 where available.

## Replay and recording (`capture::clip`)

`.ahclip` format, used by tests and the `cli record` command:

| Part | Content |
|---|---|
| Header (JSON, length-prefixed) | `{ version: 1, width, height, region, refresh_hz, backend, recorded_at, profile_snapshot, notes }` |
| Frames | Repeated: `pts_ns u64`, `seq u64`, `len u32`, zstd(BGRx region bytes) |
| Index | Offsets for seeking, at the end, with a trailer magic `AHCLIP01` |

- `ReplaySource` implements `FrameSource`, yields frames with their recorded pts. With a virtual `Clock` it runs as fast as the CPU allows; with the real clock it paces to pts.
- `ClipRecorder` taps the ring (copy on the capture thread is not allowed; the scan thread hands frames to a recorder channel with a bounded queue and counts drops).
- Labels sidecar `<clip>.labels.json`: optional per-note ground truth `{ lane, cross_pts, hold_end_pts, judgment_readback_ms }`.

## M2 notes (2026-11-21)
- Byte layout, little-endian: `u32 header_len`, header JSON, frames (`u64 pts_ns, u64 seq, u32 len, zstd level 1`), index (`u64` offset per frame), `u64 frame_count`, `AHCLIP01`. A clip without the trailer (recording killed) is rejected.
- `ReplaySource` reports the end of the clip as `SourceLost`. `set_region` crops inside the recorded region.
- `arrowhead-cli import <video> -o <clip> --region x,y,w,h [--start s] [--seconds s]` converts M0 screen recordings through ffmpeg (bgr0 rawvideo); pts = start + frame number / fps, so it suits constant-rate recordings only.
- The portal session and its D-Bus connection are kept open for the life of the source (v2 dropped them after the handshake).
- Frame pts comes from `SPA_META_Header`, requested with a `ParamMeta` update after the format is negotiated.
- Size: a 840x1440 playfield crop at 165 fps is about 42 MB/s on disk; lane-strip regions will be far smaller.

## Acceptance
- M2: `arrowhead-cli capture --seconds 10` prints fps, interval and pts-age percentiles from the real portal on KDE and Hyprland; `record` writes a clip that `replay` reads back bit-exact.
- Capture of a 4 x 60 x 900 px region holds the monitor refresh rate with `copy_us_p99 < 500`.
- Sober issue #2027 (16 fps while focused) is checked during M0 and its result written into [12](12-open-questions.md).
