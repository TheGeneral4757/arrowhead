---
created: 2026-11-21
status: draft
milestone: v0.2 (Max only in v0.1)
---

# 06 Player model and accuracy controller

Makes arrowhead play like a person at a chosen accuracy. Replaces v2's skill curve. Lives in `arrowhead-core::style`, pure and seeded.

## Judgment windows (unverified, config `[game.windows_ms]`)
| Judgment | Max abs error (ms) |
|---|---|
| Sick | 45 |
| Good | 80 |
| Ok | 130 |
| Bad | 230 |
| Miss | beyond Bad, or not pressed |

## Player model
For note `n` at song time `t`:

```
sigma_n = sigma0 * k * spread * (1 + chord*c_n + jack*j_n + stream*s_n) * (1 + fatigue * t / T)
mu_n    = bias + drift(t)
e_n     ~ mixture: (1 - p_slip) * Normal(mu_n, sigma_n) + p_slip * Normal(mu_n, 3 * sigma_n)
miss_n  ~ Bernoulli(miss_floor + miss_k * (c_n + j_n + s_n) / 3)
```

| Term | Meaning | Source |
|---|---|---|
| `sigma0` | Base spread (ms) for the style | Preset |
| `k` | Controller gain, starts at 1 | Controller |
| `c_n` | 0 single, 0.5 two-note chord, 1 three or more | Chord size from plans with the same `press_at` (+/- 5 ms) |
| `j_n` | `clamp((150 - gap_ms) / 100, 0, 1)` for the gap to the previous note in the same lane | Plans |
| `s_n` | `clamp((nps - 6) / 10, 0, 1)` over a 1 s window | Plans |
| `T` | 180 s reference song length | Constant |
| `drift(t)` | Ornstein-Uhlenbeck: `tau = 8 s`, stationary sd = `focus_drift * 20 ms` | Seeded RNG |
| `p_slip` | 0.02 | Preset |
| Release error | `Normal(release_bias, hold_release_ms)`, clamped so a hold never ends before 60% of its length | Preset |

- A missed note is not pressed at all (no ghost press). Holds that miss also skip the trail.
- Error is applied in [04](04-timing.md) as `style_error`. The model sees only planned notes (lookahead), so chord/jack/stream terms are known before the press.
- RNG: `ChaCha8Rng` seeded from `(session_seed, note_index)` so a replayed session is reproducible.

## Accuracy controller
Goal: final song accuracy within +/- 0.5% of `target`, without visible sudden changes in play.

1. **Feed-forward.** Expected accuracy for a given `k` is computed by integrating the error distribution over the judgment windows and an accuracy weight per judgment (`[game.accuracy_weights]`, unverified, default Sick 1.0, Good 0.75, Ok 0.5, Bad 0.25, Miss 0; M0 found the game's accuracy is not count-weighted (likely per-hit ms, [12](12-open-questions.md) G2), so this is only a starting guess and feedback from the HUD readout does the real work; refit from per-hit ms readbacks in v0.2). Solve for the `k` that hits `target` before the song starts (bisection, k in [0.2, 4]).
2. **Feedback.** Measured accuracy so far: the HUD accuracy readout if readable, else our own estimate from planned errors + readback. Projected final = measured weighted by notes played plus the feed-forward expectation for the rest. PI on `log k`: `Kp = 0.4`, `Ki = 0.05 per s`, update at most every 2 s, `k` changes at most 10% per update.
3. **Misses** are not a controller knob; the miss rate comes from the style only.
4. **Max** bypasses the model: `style_error = 0`, no misses, no drift, controller off.

## Presets (`[style]`, user-editable, stored in `styles/*.toml`)
| Preset | target | sigma0 | bias | spread | chord | jack | stream | fatigue | focus_drift | miss_floor | hold_release_ms |
|---|---|---|---|---|---|---|---|---|---|---|---|
| Max | 100 | 0 | 0 | - | - | - | - | - | - | 0 | 0 |
| Sweaty | 96 | 14 | +3 | 1.0 | 0.3 | 0.4 | 0.3 | 0.2 | 0.15 | 0.4% | 18 |
| Casual friend | 88 | 26 | +8 | 1.0 | 0.6 | 0.8 | 0.5 | 0.3 | 0.3 | 1.5% | 30 |
| Rusty | 82 | 30 | +15 | 1.1 | 0.5 | 0.6 | 0.6 | 0.6 | 0.4 | 2.0% | 35 |

Numbers are first guesses for M0 tuning, not facts.

UI: target slider + preset picker up front; everything above in the Advanced drawer ([07](07-app-ui.md)). Changing any advanced value makes the preset "Custom (based on X)".

## Match my friend
- Reads the opponent's score (and accuracy, if the HUD shows it) from the HUD ([03](03-detection.md)).
- `target = opponent_accuracy + margin` (margin -3% to +3%), clamped to [70, 100]. If only scores are readable: same controller on score ratio instead of accuracy, with `margin` in percent of their score.
- If the opponent's numbers are unreadable for 5 s, hold the last target and show a warning.

## Self-check (shown on Live and in History)
- Hit error from readback, or from planned error when there is no readback.
- UR = sd(hit error) x 10. Mean hit error. Histogram (Live > Hit error).
- Flags: `uniform-like` if a KS test against a uniform distribution on the observed range has p > 0.05 for 200+ notes; `too-tight` if UR < `ur_min` for a non-Max preset; both shown as warnings.

## Statistical tests ([09](09-testing.md))
- For each preset and seeds 0-49: simulated 1000-note charts give mean error within +/- 1 ms of `bias`, UR within 10% of the model's prediction, and judgment counts within a 99% binomial interval of the analytic expectation.
- Controller: from k=1, simulated songs of 300, 800 and 1500 notes end within +/- 0.5% of target for targets 85, 90, 95, 98.
- Determinism: same seed, same chart -> identical error sequence.

## v0.2 status (style track)
Built: `core::style` (presets, `Style` with serde, `StylePlayer` = model + controller + self-check, `Judge` feed-forward with bisection, `list_styles` / `save_style` / `delete_style` / `find_style` for `styles/*.toml`, `custom_name`), planner wiring (`Planner::set_style`, one draw per planned note when it is first confirmed, misses skip press and trail, hold release error clamped to 60% of the hold), runner reads `config.toml` `style`. Max is not wired in at all, so v0.1 behaviour is unchanged.

Departures and choices the spec left open:
- `miss_floor` and `miss_k` are fractions (0.004 = 0.4%); `miss_k` (extra miss rate at full chord/jack/stream load: Sweaty 1%, Casual 3%, Rusty 4%), `p_slip` (0.02), `release_bias` (= `bias`) and `ur_min` (50) are new preset fields.
- Controller error is in accuracy points (`Kp` 0.4 per point). `k` is shared by the feed-forward start value and the PI term (`log k = log k0 - (Kp e + Ki integral e dt)`, step-limited to 10%, one update per 2 s of song time). The controller needs a song length: `StylePlayer::new(.., expected_notes)` (runner uses 600) and `set_total_notes`.
- Without HUD readback the controller uses its own estimate: the planned error of each note classified by the judgment windows. `feed_accuracy(Some(acc))` overrides it.
- The feed-forward includes drift (added in quadrature) and the observed chart load (running mean of the chord/jack/stream/fatigue multiplier and miss rate).
- Tests: the +/- 1 ms mean check is pooled over the 50 seeds (OU drift makes one seed's mean wander by up to ~2 ms for Rusty); the judgment-count interval is 3.5 sd, not 99% binomial, because drift correlates neighbouring notes. The controller test skips targets the style's own miss rate makes unreachable (k pinned at 0.2) and allows +/- 0.9% at 300 notes: the last 2 s of a song cannot be steered and are about 0.4% of noise (1 sd) there; 800 and 1500 notes hold +/- 0.5%.
- Uniform-like KS test uses the last 1024 pressed-note errors.
