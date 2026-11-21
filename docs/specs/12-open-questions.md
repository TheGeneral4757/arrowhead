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
| G2 | Accuracy formula | weights 1/0.75/0.5/0.25/0 | Fit weights to 5+ results screens with known counts | |
| G3 | Does the ms readout exist on this version, is it a setting, where is it? | assumed present | Look in a song and in settings | |
| G4 | ms readout sign (is positive late?) | +1 = late | Press 50 ms late on purpose | |
| G5 | Default keybinds: do both A S W D and arrows work? | A S W D | Press each in a song | |
| G6 | HUD elements present: accuracy, judgment counts, scores for both players, opponent accuracy? | search all | Screenshot during a song | |
| G7 | Scroll speed setting range; can speed change mid-song? | fitted live | Settings menu; a song with known changes if any | |
| G8 | Hold trail look; does the trail shrink as it is held? | learned | Record a hold | |
| G9 | Is a key press of 35 ms always registered? Minimum? | 35 ms | Lower until misses appear | |
| G10 | Does the player choose a side, and does it stay the same per server? | detected | Play two rounds | |
| G11 | Results screen: is there a stable element to detect? | receptors gone 1.5 s | Record the end of a song | |
| G12 | Do receptors light up on press (side detection relies on it)? | assumed yes | Record presses | |
| G13 | Upscroll default or downscroll default? | detected | Settings | |

## Platform
| # | Question | Default | How | Answer |
|---|---|---|---|---|
| P1 | Sober 16 fps while focused (issue #2027) on this box, KDE and Hyprland | unknown, blocks accuracy | Run a song, watch Sober's FPS | |
| P2 | PipeWire pts and SHM copy time at full refresh | measure | `cli capture` (M2) | |
| P3 | Does Sober receive uinput keys reliably (issues #1959, #1940)? | yes per v2 | Key test in Setup | |
| P4 | Layer-shell overlay above fullscreen Sober on KWin and Hyprland | unknown | Spike | |
| P5 | KWin script focus probe sees Sober's class | unknown | Spike | |
| P6 | GlobalShortcuts portal works on this KDE and Hyprland setup | unknown | Spike | |
| P7 | Windows: does Roblox's anti-cheat block SendInput or capture? | unknown | v0.2 test on a spare account | |
| P8 | Portal window-source restore across Sober restarts | unknown, monitor default | Spike | |
