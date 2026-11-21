---
tags: [research, funky-friday, detection]
created: 2026-11-21
---

# Funky Friday reference imagery

> [!warning] Basic examples only
> These images are rough examples (mostly old YouTube thumbnails). They do not show what the game looks like now or on any given setup. Do not take positions, sizes, colours, shapes or HUD layout from them; arrowhead learns all of that from the user's screen.

Folder: `docs/research/ff-refs/` (copied from the planning scratchpad).

Source limits: roblox.fandom.com, funkyfriday.fandom.com and other fandom wikis return 403; sportskeeda returns 405; the Roblox games API needs auth. So no fandom images were usable. Almost all gameplay frames come from YouTube maxresdefault thumbnails (1280x720 stills, not extracted video frames). Frame size is 1280x720 in every gameplay still. Players appear to use a mobile-style touch overlay in some frames (big white circles, grey sliders).

## Files

| File | Source URL | What it shows |
|---|---|---|
| yt-FkTQNQjbf1I.jpg | https://img.youtube.com/vi/FkTQNQjbf1I/maxresdefault.jpg (video: https://www.youtube.com/watch?v=FkTQNQjbf1I) | Gameplay, Doomsday (Hard), dark red room. Best detection frame in the set. |
| yt-uMAp_-fVvE8.jpg | https://img.youtube.com/vi/uMAp_-fVvE8/maxresdefault.jpg | Gameplay, Banned (Hard), Portuguese UI ("Pontuacao"), grey/black/pink/green notes, four-lane playfield on black. |
| yt-yTX0nzxKP8k.jpg | https://img.youtube.com/vi/yTX0nzxKP8k/maxresdefault.jpg | Gameplay, Markov (Hard), [BOT] label, purple room. Sick popup, ms readout, stats box. |
| yt-mlIIuKT27zk.jpg | https://img.youtube.com/vi/mlIIuKT27zk/maxresdefault.jpg | Gameplay, Left Unchecked (Hard), PFC. Arrow-style (FNF) notes on black playfield. |
| yt-JYzTTJrlFkM.jpg | https://img.youtube.com/vi/JYzTTJrlFkM/maxresdefault.jpg | Thumbnail "4 YEARS" with a small gameplay crop at left (four columns of dot notes, hollow receptor rings at bottom). Lane geometry only. |
| yt-szvsLmuL6KE.jpg | https://img.youtube.com/vi/szvsLmuL6KE/maxresdefault.jpg | Thumbnail "TOP 5 HARDEST SONGS" over gameplay. Shows score at top corners and dot notes in four lanes. |
| yt-DSncbhJi7bw.jpg | https://img.youtube.com/vi/DSncbhJi7bw/maxresdefault.jpg | Song-select menu ("Map Selection", FC title). No gameplay. Useful for the menu layout only. |
| yt-995mghY-l0Y.jpg | https://img.youtube.com/vi/995mghY-l0Y/maxresdefault.jpg | Title card "Funkdela Catalogue, All Song FC". Not gameplay. |
| yt-_mvBGvbZ6eM.jpg | https://img.youtube.com/vi/_mvBGvbZ6eM/maxresdefault.jpg | Title card "HARDEST SONG?" over blurred game world. Not gameplay. |
| roblox-thumb-og-500x280.jpg | https://tr.rbxcdn.com/180DAY-c5ce5e8aba8b6393a36d211c35ae6f57/500/280/Image/Jpeg/noFilter (og:image on https://www.roblox.com/games/6974384350) | Promo thumbnail with a ":RICH" speech bubble. Not gameplay. |
| wikiwiki-header-funky-friday.jpg | https://cdn.wikiwiki.jp/to/w/roblox-info/Funky%20Friday/Header/::attach/Funky%20Friday.jpg | Key art (Funky Friday logo, FNF-style characters). Not gameplay. |

## Visual observations for detection (1280x720 frames)

### Lanes and receptors
- Four lanes, all frames. Receptors sit near the bottom, about y=620 to 650 of 720.
- Doomsday frame: receptor centres at x about 448, 575, 703, 831, y about 632, radius about 50 px. White outlines.
- Banned frame: receptor centres at x about 418, 567, 715, 862, y about 622. Grey filled circles with a thin white rim. Lane spacing about 148 px. Playfield is a black column about x 350 to 925.
- Markov frame: receptors at the same x as Doomsday (448, 575, 703, 832), y about 630. Grey-white circles.
- Left Unchecked frame: arrow receptors (light blue, FNF style) in a black column about x 380 to 900. Lane order left to right is L, D, U, R (x about 460, 580, 700, 815). Lane width about 120 px.
- Four-column dot style (JYzT, szvs thumbnails): hollow white rings at the bottom, filled dots in each column.
- Notes are filled circles in the Roblox-style modes (about 90 to 100 px diameter in 1280x720), not arrows. Left Unchecked is the exception.

### Direction
- Every gameplay frame shows notes falling toward receptors at the bottom, so default is downscroll. No upscroll frame was found in the set.

### Note colours (approx, from frames)
- Magenta/pink: about #ff1fa8 to #ff00b4.
- Purple: about #7000ff to #8a2be2.
- Cyan/light blue: about #3ab0f0 to #6fd8ff.
- Green: about #00ff88.
- White receptor rims: #ffffff.
- Hold trails: not seen in any frame. The gray (85,85,85) trail colour from CLAUDE.md is not verified here.

### Judgment popup
- Shown in the bottom corner of the playfield, not centred. Doomsday, Markov: bottom-left, about x 265 to 375, y 615 to 700. Left Unchecked: bottom-right, about x 890 to 1000, y 560 to 690.
- Style: stacked "Sick!!" words with a rainbow gradient (green, yellow, orange, pink). Stacked copies are visible.
- Ms readout directly under the popup, small white text with dark outline. Examples: "-5.59 ms" (Doomsday), "4.15 ms" (Markov), "-0.02 ms" (Left Unchecked). Signed, two decimals.
- Rating names seen: Sick. The side stats list also shows Good, Ok, Bad, Missed. "Bad" is a real tier here, not only Miss.

### Text and HUD
- Score: top-left and top-right, one per player. Bold white Roblox-style font with dark outline. Example: "Score: 138500" and "Score: 120780".
- Title: top-centre, song name in colour (red for Hard in Doomsday/Markov, green in Banned), then credits, then timer ("01:25").
- Stats box: right side, stacked lines "Sick: 347 / Good: 0 / Ok: 0 / Bad: 0 / Missed: 0". Small white text.
- Bottom centre: "Accuracy: 100.00% | Misses: 0 | Combo: Full Combo (347)". Small white text, dark outline.
- Top-left corner has Roblox UI icons (chat count, settings, etc.).
- Touch overlay in some frames: two grey vertical sliders at top-left and big white circles over the playfield. Not present in all frames.

### Song select (yt-DSnc)
- Dark panel, four sort buttons at the bottom ("Highest Score", "Least Misses", "Highest Accuracy"). Song list with colored titles (pink, yellow, red, green) and a blue-bordered "Select" button per song. Right panel shows the chosen song, and "Currently Selected / Opponent's Selection" lines.

### Not covered by the set
- Countdown, results screen, upscroll, hold notes with trails, and arrow skins: no usable frame found. These need a new search or a live capture of the user's own game.

## Recommended next step
- Extract frames from one or two videos at 1080p or higher for real playfield frames, using yt-dlp outside this subagent. Good candidates: FkTQNQjbf1I (Doomsday Hard FC), yTX0nzxKP8k (Markov Hard FC).

## YouTube gameplay videos for later frame extraction

Search results give no upload dates or resolution, so 1080p and recency are not verified. Most results were described as 2021 to 2022 uploads, so check the upload date before relying on them.

- https://www.youtube.com/watch?v=FkTQNQjbf1I - [ROBLOX] Funky Friday (Doomsday Remastered) Hard FC, full song
- https://www.youtube.com/watch?v=yTX0nzxKP8k - Funky Friday - Markov (FC) Full Song (Update)
- https://www.youtube.com/watch?v=mlIIuKT27zk - FUNKY FRIDAY - Left Unchecked (HARD) Perfect FC 100%
- https://www.youtube.com/watch?v=DSncbhJi7bw - Roblox Funky Friday - All FNF Songs (Hard) FC
- https://www.youtube.com/watch?v=uMAp_-fVvE8 - Funky Friday ROBLOX
- https://www.youtube.com/watch?v=995mghY-l0Y - [Funky Friday] All Song FC (Funkdela Catalogue)
- https://www.youtube.com/watch?v=_mvBGvbZ6eM - New HARDEST Song in Funky Friday Roblox!
- https://www.youtube.com/watch?v=JYzTTJrlFkM - IT TOOK ME NEARLY 4 YEARS TO FC THIS (Funky Friday)
- https://www.youtube.com/watch?v=szvsLmuL6KE - TOP 5 HARDEST SONGS IN FUNKY FRIDAY (Roblox)
- Playlist: https://www.youtube.com/playlist?list=PLFZPTfIssTf67Ua1Sj7M6VS0AUU5Ob_w1 (Hardest-difficulty WIP playlist)
