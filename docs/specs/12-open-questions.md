---
created: 2026-11-21
status: open
---

# 12 Open questions

Facts the specs depend on that nobody has verified. Each one is a config value with a default until answered. Answer in M0 ([roadmap](11-roadmap.md)), then update the default, the spec, and this table.

The reference images in `docs/research/ff-refs/` are rough examples, not evidence for any row here.

## Game
| # | Question | Current default | How to answer | Answer |
|---|---|---|---|---|
| G1 | Judgment windows (Sick/Good/Ok/Bad) | 45/80/130/230 ms | Deliberate offset presses with readback; find where the judgment changes | |
| G2 | Accuracy formula | weights 1/0.75/0.5/0.25/0 | Fit weights to 5+ results screens with known counts | Score is exact: 350/200/100/50 per Sick/Good/Ok/Bad, -30 per Miss (Gospel: 243/177/123/115/69 -> 136430; also 654 misses -> -19620). Checked on 3 screens (Gospel, Manifest, Deathmatch). Accuracy is not count-weighted: no monotonic weights fit all three, and the exact solve puts Bad above Ok. Likely per-hit ms-based. Read accuracy from the HUD; do not predict it from counts |
| G3 | Does the ms readout exist on this version, is it a setting, where is it? | assumed present | Look in a song and in settings | Yes, always on: signed "n.nn ms" under the judgment popup (ours in our playfield, opponent's near theirs). Setting not checked |
| G4 | ms readout sign (is positive late?) | -1 (positive = early) | Press 50 ms late on purpose | Confirmed by user: late = negative ms, early = positive. `readback_sign = -1` |
| G5 | Default keybinds: do both A S W D and arrows work? | A S W D | Press each in a song | Both A S W D and arrows work (user). A S W D shown under receptors at song start |
| G6 | HUD elements present: accuracy, judgment counts, scores for both players, opponent accuracy? | search all | Screenshot during a song | Yes: stats panel left (Accuracy, Highest Combo, Combo, Hits, Misses, Mean MS, KPS), judgment counts right, both scores bottom corners, health bar + hit-error bar bottom centre, title/time top centre. No opponent accuracy seen |
| G7 | Scroll speed setting range; can speed change mid-song? | fitted live | Settings menu; a song with known changes if any | Song menu has Rate (1.00x), SV charts and Modcharts toggles (on), so speed can change mid-song. User: speed varies per song, so it must be fitted live, never a constant |
| G8 | Hold trail look; does the trail shrink as it is held? | learned | Record a hold | Grey in every lane, shrinks into the receptor while held (user) |
| G9 | Is a key press of 35 ms always registered? Minimum? | 35 ms | Lower until misses appear | |
| G10 | Does the player choose a side, and does it stay the same per server? | detected | Play two rounds | Ours is always the larger, centred playfield; opponent smaller on the right (user, 2026-11-21) |
| G11 | Results screen: is there a stable element to detect? | receptors gone 1.5 s | Record the end of a song | Yes: "Stats from last song" panel (accuracy, mean ms, combos, counts, total notes, final score) |
| G12 | Do receptors light up on press (side detection relies on it)? | assumed yes | Record presses | Yes: grey receptors light up in the lane colour on press |
| G13 | Upscroll default or downscroll default? | detected | Settings | User plays upscroll only. Keep detection; downscroll stays supported but untested |

## Platform
| # | Question | Default | How | Answer |
|---|---|---|---|---|
| P1 | Sober 16 fps while focused (issue #2027) on this box, KDE and Hyprland | unknown, blocks accuracy | Run a song, watch Sober's FPS | KDE: no. Steady 165 fps focused (Roblox stats and 1650 unique frames in 10 s of video, 2026-11-21). Hyprland: user reports near-identical fps from past play and will not run arrowhead there |
| P2 | PipeWire pts and SHM copy time at full refresh | measure | `cli capture` (M2) | |
| P3 | Does Sober receive uinput keys reliably (issues #1959, #1940)? | yes per v2 | Key test in Setup | |
| P4 | Layer-shell overlay above fullscreen Sober on KWin and Hyprland | unknown | Spike | |
| P5 | KWin script focus probe sees Sober's class | unknown | Spike | |
| P6 | GlobalShortcuts portal works on this KDE and Hyprland setup | unknown | Spike | |
| P7 | Windows: does Roblox's anti-cheat block SendInput or capture? | unknown | v0.2 test on a spare account | |
| P8 | Portal window-source restore across Sober restarts | unknown, monitor default | Spike | |
