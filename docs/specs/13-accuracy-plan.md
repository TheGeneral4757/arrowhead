---
created: 2026-11-24
status: in progress
---

# 13 Accuracy plan (v0.2 fixes from the 2026-11-24 clips)

Findings from six live runs recorded on 2026-11-24 (`~/ah-clips/song-1124-*.mkv`: Accelerant, Deathmatch, Pico vs Matt, Manifest, Gospel, Ghost 2.00x), compared against the game's own HUD and results screens. This is the work order. It slots into [11 Roadmap](11-roadmap.md) before draft PR #12 (v0.2) merges: these fix v0.2 features (HUD reader, readback, overlay) that were untested against the game.

## What the clips showed

| Run | Overlay | Sick / Good / Ok / Bad / Miss | Results Mean MS | Offset |
|---|---|---|---|---|
| Accelerant | on | 614 / 0 / 0 / 7 / 33 | -3.52 | +0.48 |
| Deathmatch | on | 688 / 0 / 0 / 0 / 0 | -3.39 | +0.48 |
| Pico vs Matt | off | 709 / 0 / 0 / 0 / 28 | -4.21 | +2.58 |
| Manifest | off | 488 / 0 / 0 / 0 / 0 | -5.22 | +2.58 |
| Gospel | on | 688 / 0 / 16 / 2 / 21 | -5.39 | +2.58 |
| Ghost (2.00x) | off | 4696 / 15 / 48 / 9 / 193 | -8.88 | +2.58 |

- Misses, not timing, decide accuracy: Deathmatch is 100% at -3.39 ms, Pico's 96.20% is exactly 709/737.
- Overlay on: misses all song long. Overlay off: clean after settling (the user also got perfect overlay-off runs of Accelerant and Gospel). Pico's 28 and Ghost's 185 early misses were a restart and a user mistake, not detection.
- Presses are about 4 ms late in real time. The ideal offset is about -2.5 ms; the saved +2.58 ms got there through the readback bug below.
- Remaining overlay-off misses (Ghost 65.5-68.5 s): three hold heads in a row pass unlit receptors live, while an offline `detect --plan` of the same frames plans each press on time.
- 14% of frames in that window repeat the previous frame exactly.

Game facts found (now in [12](12-open-questions.md)): the stats panel's Mean MS has the opposite sign to the ms popup and the results screen (G14); at a song rate other than 1.00x the game's ms are in song time (G15).

## Order

Each phase ends with its check. Phases 1 and 2 must land before PR #12 merges.

### Phase 1: stop feeding bad numbers to timing (v0.2 blocker)
Status (2026-11-25): built. 1.2 is `core/src/hud/guard.rs`: a contradiction must hold on 3 stable reads (same numbers two ticks running); it then restores the stats face from the last read that checked out if something was taught since, else forgets the stats face so OCR relearns it. 1.4: the atlas file version is now 2 (old files are ignored); the dev box's `game_bias_ms` was set to -2.5 by hand (backup `default.toml.bak-1125`).

| # | Change | Where | Done when |
|---|---|---|---|
| 1.1 | Mean MS sign: new `[game] stats_mean_sign = -1` (config, not a constant). Apply it in `teach_ms_at` / `teach_ms_first_hit` and `hit_ms_from_mean` | `core/src/hud/mod.rs`, `runner/src/ocrteach.rs`, `core/src/config.rs`, spec 08 | Teaching on a frame with popup "-2.13 ms" learns the `-` glyph and reads -2.13 |
| 1.2 | Atlas consistency guard: after any teach, require Hits = Sick+Good+Ok+Bad, the two Miss counters equal, counts never decreasing; on a contradiction roll back that teach and relearn the face | `runner/src/hud.rs`, `core/src/hud/` | A unit test with a mis-taught digit rolls back; `cli hud` on `pico60.png` reads 84.53 / 153 / 28 |
| 1.3 | Offset save gate: save only with >= 30 readbacks and a small spread (MAD), and move at most 1 ms per song | `runner/src/hud.rs:388` (`save_bias`) | 3 readbacks no longer change `game_bias_ms` |
| 1.4 | One-off reset: release note tells the user to set `game_bias_ms` to -2.5 and delete `glyphs/default.bin` (or bump the atlas file version so it is ignored) | atlas loader, CHANGELOG highlight | A fresh run relearns the font |
| 1.5 | Results Mean MS as a strong readback sample: the whole-song mean from the results screen (font-free, every hit) nudges the saved offset when it passes the checks in 4.3 | `runner/src/hud.rs` (`finish_song`) | Offset converges to the results-screen mean over 2-3 songs |

### Phase 2: the overlay must not touch what we read (v0.2 blocker)
Status (2026-11-25): built. The run overlay is one panel (pill, judgment, error bar) placed clear of the playfield and every HUD region; lanes, receptors, ticks and outlines need `--debug`. Settings' overlay select now acts at once like Live's toggle. Check on a real desktop: `detect` on a clip recorded with the overlay on.

| # | Change | Where | Done when |
|---|---|---|---|
| 2.1 | Draw nothing over the lanes, receptors or HUD regions during a run: no lane strips, receptor circles, planned ticks or region labels. Keep the status pill, judgment and error bar in free screen space | `overlay/src/render.rs`, spec 07 | `detect` calibrates on a clip recorded with the overlay on |
| 2.2 | No "TITLE" (or any) label near HUD text; outlines only in a debug mode that is off during runs | `overlay/src/render.rs` | Titles OCR without "TITLE" |
| 2.3 | Settings overlay toggle starts or stops only the overlay child (like Live's show/hide), never the run | `app/src/` commands | Toggling mid-song costs no notes |
| 2.4 | Minimise only on a single monitor in Auto (done, uncommitted) | `app/src/overlay.rs` | - |

Later option: capture the Sober window instead of the monitor (open question P8), which removes the overlay from the frames entirely.

### Phase 3: tools to see live misses
| # | Change | Where | Done when |
|---|---|---|---|
| 3.1 | `detect --profile`: replay a clip with the saved layout and colours, the way a live run starts | `cli/src/detect.rs` | Mid-song dense clips replay without calibrating |
| 3.2 | Flight recorder: ring of the last ~10 s of frames (playfield region) plus planned and sent presses; dump an `.ahclip` + press log when the Misses counter rises. Off by default (`[debug] flight_recorder`) | `runner/`, `capture/src/clip.rs` | A live miss produces a clip that `detect --profile --plan` replays, with the sent presses side by side |

### Phase 4: fix what the tools find, and timing polish
| # | Change | Where | Done when |
|---|---|---|---|
| 4.1 | Hold heads skipped live (Ghost 65.5-68.5 s): find the live-only cause with 3.1/3.2 (suspects: saved colours vs fresh, Max style planning, readback corrections, scheduler) and fix it | `core/src/plan.rs`, `track.rs`, `sched.rs` | The Ghost hold section replays with every head pressed and held |
| 4.2 | Stale frames: skip a track observation when the frame repeats the last one (no head moved), so an old position never gets a new pts | `core/src/track.rs` | Synthetic test with 15% duplicated frames keeps crossing error < 1 ms; real mean lateness drops |
| 4.3 | Readback gating: adapt only from readings where the popup agrees with the panel's Mean MS change on a +1 Hits step | `runner/src/hud.rs`, `ocrteach.rs` | Wrong-sign or misread popups never reach `Bias` |
| 4.4 | Song rate: parse "(N.NNx)" from the title; divide game ms by the rate before readback and adaptation; store offsets in real ms | `runner/src/ocrteach.rs`, `hud.rs`, spec 04 | Ghost at 2.00x adapts to about the same real offset as 1.00x songs |
| 4.5 | Per-lane / per-pattern corrections only after ~20 good samples each | `core/src/adapt.rs` | Early-song corrections stay at 0 |

### Phase 5: OCR you can trust
| # | Change | Where | Done when |
|---|---|---|---|
| 5.1 | Digit look-alike map in number fields (S/5, O/C/0, Y/4, I/l/1, B/8) and minus sign recovery from the panel sign rule | `runner/src/ocrteach.rs` | The six results screens read their counts and score |
| 5.2 | Results voting: 3-5 frames x all variants, vote per field | `runner/src/ocrwork.rs` | Same |
| 5.3 | Accept results only if Total = sum of counts and score = 350S + 200G + 100O + 50B - 30M (exact on every screen checked); use the two equations to fix a single bad field | `runner/src/hud.rs` | "240804" and "TOTAL NOTES 788" are corrected or rejected |
| 5.4 | Titles: split name / "By:" / rate; fuzzy-match against names in `songs.json`; merge the garbled entries already saved | `runner/src/ocrteach.rs`, `core/src/adapt.rs` | The same song played twice gets one memory entry |
| 5.5 | Accuracy unit: history stores 1.0 for Deathmatch's 100% and 55.33 elsewhere; store percent everywhere | `runner/src/hud.rs` | History rows agree on units |

### Phase 6: faster, safer starts
| # | Change | Where | Done when |
|---|---|---|---|
| 6.1 | Keep learned colours and the last scroll speed across a restart in the same session | `runner/src/lib.rs` | Restart mid-song plays within 1 s |
| 6.2 | Arm on the song menu; learn the lane background from the countdown frames before notes appear | `runner/src/lib.rs`, `core/src/detect/calib.rs` | First note of a song is hit from a menu start |

## Then the rest of v0.2 and later
1. Re-record the same six songs with the fixes (overlay on and off); compare against the table above.
2. Remaining v0.2 work from the roadmap: Windows checks (P7, P9-P13), manual checklist rows 10-11, then merge PR #12 and release.
3. v0.3 and later as listed in [11](11-roadmap.md); capturing the game window (P8) moves up if 2.1 is not enough.

## Open
- G2: Accelerant's 98.71% does not fit its counts; the accuracy formula is still unknown.
- Which teaching step mis-taught the stats digits (1.2 guards against it either way).
