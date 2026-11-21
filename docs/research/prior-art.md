# Prior art: rhythm-game auto-players and analysis tools

Scope: external screen-reading / analysis approaches and humanization models. Lua script-injection executors skipped on purpose.
Confidence: "read" = README fetched; "snippet" = only search description seen, repo not opened.

## Funky Friday / Friday Night Funkin' projects

### Funkin' Bot (ArkCod3/Funkin-Bot) - read
- URL: https://github.com/ArkCod3/Funkin-Bot
- Language: Python
- Detection: image captures of 4 arrow boxes; per-arrow colour sampling. Config JSON made by `make_config.py` (pause the level, hover each arrow, shift to confirm, bot extracts arrow colours).
- Timing: no explicit humanization. Latency is tuned by where along the arrow box you sample: sample lower if inputs lag, higher if early.
- Humanization: none found in README.
- Strengths: works on any resolution, browser and installed versions, supports mods via configs (vanilla and netmods shipped), has a calibration flow.
- Weaknesses: struggles with very fast consecutive notes; fails when mod idle and note colours match.
- Failure modes: manual start/stop per song; requires notes not to be near the arrow box when calibrating.

### Friday Night Funkin Bot (MarkConstantine) - read
- URL: https://github.com/MarkConstantine/Friday-Night-Funkin-Bot
- Language: Python (`play.py`, `arrow.py`)
- Detection: colour check on fixed arrow positions.
- Humanization: none.
- Weaknesses: start/stop with `~` manually per song; no config tooling seen.

### Fnf_bot (vantonuk35) - read
- URL: https://github.com/vantonuk35/Fnf_bot
- Language: C++ with OpenCV
- Detection: colour checking of arrows.
- Constraints: window must be centred on screen; default arrow colours only (custom colours break it); game window name must match (WinSpy++ to fix).
- Humanization: none.

### Friday-Night-Funkin-bot (RavenEbonscale) - read
- URL: https://github.com/RavenEbonscale/Friday-Night-Funkin-bot
- Language: C++
- Control: numpad 1 starts, numpad 0 stops. README is thin; no detection detail.

### Funky Friday AutoPlay (trxshonzaza) - read
- URL: https://github.com/trxshonzaza/Funky-Friday-AutoPlay
- Language: C#
- Detection: Win32 GDI32 `GetPixel` at fixed coordinates.
- Constraints: 1920x1080 only, maximised windowed only, Player 2 (right side) only, MiddleScroll and DownScroll unsupported, default note skin recommended, Windows only.
- Known limits (own README): cannot hold long notes, delayed inputs on low-end machines, can miss notes.
- Humanization: none.

### Estrol/funky-friday-autoplay, OldNCP and other Lua forks
- Lua script-injection family. Skipped per scope.

### Our own project (fnf-thing-v2) for comparison
Already has: per-OS capture and input backends, a 3x3 neighbourhood colour match, edge-triggered presses, a rating-tier humanizer tied to FNF judgement windows (45/80/130 ms), skill slider, timing offset, and a calibration page. The public bots above mostly lack the humanizer and calibration UX.

## osu! / osu!mania tools

### circleguard/circleguard - read
- URL: https://github.com/circleguard/circleguard
- Language: Python
- Purpose: replay viewer and cheat-signal analyser.
- Metrics: unstable rate (UR), snaps (aim-correction signal), similarity (replay stealing), frametime (timewarp), raw per-frame table.
- Relevance: the reference for an analysis-side check. Use its UR definition to score our own output.

### osu-replay-analysis family (search only, snippet)
- Includes circleguard-like forks, a LightGBM re-simulation tool (toroktomi), and an LLM-assisted analyser (zakack). Snippets only; not opened.

### osu!mania bots (search snippets only)
- lleeooaa/Osu_mania_AI: RL bot, WIP. README lists score/accuracy animation delays corrupting the reward signal, and slider vs blank reward conflicts.
- Iulian69420/osu_mania_bot: 4K only.
- RhythmSharp (rwdkor): C# fork that includes a humanizer, README says it still needs work.
- OsuBoost (lvkv): "player-piano" bot, near-perfect play, 4-key.
- Autosu (KuromeSama6), maniac (fs-c), AutOsu (gameswu): snippets only.
- Note: all are reads of descriptions, not verified code.

### StepMania autoplay
- Search returned no GitHub hits. StepMania's built-in autoplay is a game feature, not checked here.

## Reference facts (osu! wiki, WebSearch)
- Unstable rate = standard deviation of hit errors (ms), displayed x10 (tenths of ms). Measures consistency, not accuracy. Source: https://osu.ppy.sh/wiki/no/Gameplay/Unstable_rate
- Auto mod in mania may miss overlapping objects in one column or objects within 1 ms of each other. Source: https://osu.ppy.sh/wiki/Gameplay/Game_modifier/Auto
- A forum feature request notes a human hit-error histogram should look roughly normal. Source: https://osu.ppy.sh/community/forums/posts/7307425
- Gap: no source found on how osu!lazer's hit error meter differs from stable. Not resolved.

## Ideas worth stealing
1. Score our own output with circleguard-style UR: stddev of (press time minus target time) per note, x10. A humanizer that claims a human distribution should land in a plausible UR band. Log it in the stats panel.
2. Keep a per-note hit-error log and plot it as a histogram (the forum feature request). Check the shape is roughly normal and centred near the timing offset.
3. Calibration wizard in the Funkin' Bot style: pause the level, hover each arrow, confirm with a key, extract colours automatically. Our Calibration page can do the same for colours and coords.
4. Sample point along the note lane as a tunable knob. The Funkin' Bot README says lower the sample point if inputs lag, higher if early. Our `timing_offset_ms` covers this in time, but a visible knob for the capture position is cheap.
5. Per-profile JSON configs for skins and mods (netmods vs vanilla). We already have profiles.
6. Snaps metric from circleguard as a sanity check on any aim-like or timing-spike output.

## Pitfalls to avoid
1. Hard-coded resolution and window position. Every public bot breaks on the first change. Calibrate per profile and per monitor, which the CLAUDE.md already requires.
2. Custom skins and mod colours break colour matching. Keep tolerance configurable and test with a non-default skin.
3. Fixed sample point vs variable scroll speed. Check it still works at high speed.
4. Long holds: most public bots cannot hold. Our trail tracking is a real differentiator, so test it, do not assume.
5. Low-end input latency. The C# bot reports delayed inputs. Measure end-to-end latency, do not assume it is zero.
6. Reward or metric corruption from UI animation (the osu!mania RL note). Sample colours only after a stable frame, not during score-popup animations.
7. Uniform random jitter reads as a bot. Use the rating-distribution model and check the resulting UR, rather than adding noise.
8. Account and ToS risk. Every project here is a warning: bans are the normal outcome on ranked play. Treat these as reference code, and keep the user's own risk decision explicit.
9. Source quality: most mania bots were only seen as search descriptions. Do not cite them as verified implementations.

## Not checked
- StepMania autoplay internals.
- osu!lazer hit-error meter source.
- RhythmSharp humanizer code.
- Any GitHub project that does a real timing-distribution model beyond uniform jitter.
