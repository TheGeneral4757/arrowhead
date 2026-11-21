# Funky Friday mechanics, delta vs report.md (May 2026). Researched 2026-11-21

Coverage warning: roblox.fandom, namu.wiki, funkipedia, TheGamer, sportskeeda and vinegarhq.org all returned 403/404/405 to fetches, and web search returned nothing official. Most items below are UNVERIFIED. No in-game screenshots were seen. In-game confirmation is needed for sections 3 and 4.

## 1. Judgment / scoring
| Item | Finding | Confidence | Source |
|---|---|---|---|
| Windows (Sick/Good/Ok/Bad/Miss ms) | No new data. No source found for any 2026 change. report.md values stand (Sick ~±45-50, Good ~±80-90, Ok ~±130, Bad ~±230 inferred). | Low | search found no official numbers; see report.md sources |
| Accuracy formula | Not found. One SEO page says points scale with distance from the centre of the window. | Very low | https://robloxfunkyfridayscore.pages.dev (SEO, unreliable) |
| 2026 patch notes | No official Funky Friday patch notes found. Nothing on judgment changes. | n/a | WebSearch returned only generic Roblox devforum recaps |
| Autoplayer chanceValues | Still Sick 96, Good 92, Ok 87, Bad 75/77 in mirrors, so no sign of a change. | Medium | https://www.scribd.com/document/831594042/main-lua, report.md |

## 2. Speed / direction / keys / binds
| Item | Finding | Confidence | Source |
|---|---|---|---|
| Scroll speed | An in-game setting. Guide advises >=1.5x. Value scale unverified. | Low-Med | https://bioblox.pages.dev/guides/roblox-funky-friday-guide |
| Scroll direction | Selectable up or down. Upscroll is the "default" per earnaldo; competitive players "mostly use up-scroll". Confirm the default in-game. | Low-Med | https://bioblox.pages.dev/guides/roblox-funky-friday-guide ; https://earnaldo.com/blog/funky-friday-free-robux-guide (search snippet) |
| Mid-song speed changes | Not found. Rate mods (PlaybackSpeed) exist per report.md. | - | - |
| Key modes 4K-9K | Not re-verified. Taken from report.md (GetKeyCount 4-9). | Med (old) | report.md |
| Default keybinds | UNCONFIRMED that ASWD and arrows both work by default. The Arrow Customization > Keybinds subtab shows each arrow's default, and the table is an image. Original FNF and many guides assume arrows plus ASWD/DFJK. The report.md claim of "arrow keys; multiple keys per lane allowed" is consistent. Confirm in-game. | Low | https://mirtheon.pages.dev/posts/how-to-change-keybinds-in-funky-friday (mirror of TheGamer); bioblox says "do not use default arrow keys", so arrows are probably the default |
| Rebinding | Arrow Customization menu: remap per arrow, multiple keys per lane, presets. | Med | mirtheon (above) |
| Other mods | Camera movement and spectators can be turned off. Useful for screen reading. | Low-Med | bioblox |

## 3. In-game UI
No reliable source found for receptor position, left/right side placement, judgment popup, ms/hit-error display, countdown, results screen, opponent score display or song title/difficulty location. Only inherited facts (report.md): "FRESH!" burst at near-zero delta, and the side is "Left" or "Right" (framework.UI.CurrentSide). The earlier claim of an optional ms overlay is unconfirmed. Capture screenshots of a private-server round to calibrate.

## 4. Skins / colours / holds
| Item | Finding | Confidence | Source |
|---|---|---|---|
| Customisation | Arrow Customization menu offers arrow style, colours and presets (save/load). Colour-based detection must be recalibrated per profile or preset. | Med | mirtheon; https://sportskeeda.com/esports/how-customize-arrows-roblox-funky-friday (search snippet) |
| Skin readability | Heavily animated skins can obscure notes. | Low | bioblox/guide snippets |
| Hold trail appearance | Nothing new found. Keep project assumption of gray (85,85,85). | - | CLAUDE.md |

## 5. Sober (org.vinegarhq.Sober) quirks
Open GitHub issues (all unresolved, updated 2026). Fetched via the GitHub API.
| Issue | Relevance | URL |
|---|---|---|
| No keyboard input / not receiving input on login (#1959, #1940, #973) | Window may need focus or a re-click. uinput virtual key delivery can silently fail | https://github.com/vinegarhq/sober/issues/1959 ; https://github.com/vinegarhq/sober/issues/1940 |
| 16 FPS when focused, 60 when unfocused (KDE Plasma 6 Wayland, CachyOS) (#2027) | Directly matches our setup. It affects capture cadence and timing. | https://github.com/vinegarhq/sober/issues/2027 |
| Text input breaks on Vulkan after fullscreen/resize (#1906) | Avoid resizing mid-session | https://github.com/vinegarhq/sober/issues/1906 |
| High ping in low-ping servers (#2524, 2026-11-16) | Network jitter. Not an input problem. | https://github.com/vinegarhq/sober/issues/2524 |
| Shift key not working in the Flatpak (#1807, closed) | Modifier handling quirk | https://github.com/vinegarhq/sober/issues/1807 |

- No Sober-specific capture issue found. The portal ScreenCast/PipeWire path is compositor-level, so Sober is not directly involved. A Manjaro thread notes black screens from Flatpak OBS under Wayland; that is OBS-specific.
- The NixOS wiki reports a working Wayland setup. https://wiki.nixos.org/wiki/Sober
- Not verified: Sober's frame pacing and its vsync/FPS cap versus the scan rate.
