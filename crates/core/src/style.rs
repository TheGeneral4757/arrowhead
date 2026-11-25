//! Player model and accuracy controller (spec 06). Pure and seeded: the planner draws one
//! `NoteDraw` per planned note. Max bypasses everything.

use std::fs;
use std::path::{Path, PathBuf};

use rand_chacha::ChaCha8Rng;
use rand_chacha::rand_core::{RngCore, SeedableRng};
use serde::{Deserialize, Serialize};

use crate::config::{ConfigError, Game, load_toml, save_toml};

/// Reference song length for the fatigue term, ms.
const T_REF_MS: f64 = 180_000.0;
const DRIFT_TAU_MS: f64 = 8_000.0;
/// Stationary drift sd per unit of `focus_drift`, ms.
const DRIFT_SD_MS: f64 = 20.0;
const K_MIN: f64 = 0.2;
const K_MAX: f64 = 4.0;
const KP: f64 = 0.4;
const KI: f64 = 0.05;
const UPDATE_EVERY_S: f64 = 2.0;
const MAX_STEP: f64 = 0.095_310_179_804_324_86; // ln(1.1)
const RING: usize = 1024;
pub const HIST_BINS: usize = 31;
pub const HIST_BIN_MS: f64 = 10.0;

/// A player style. Values the spec marks as presets; `miss_floor` and `miss_k` are fractions
/// (0.004 = 0.4%), `target` is a percentage.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Style {
    pub name: String,
    pub target: f64,
    pub sigma0: f64,
    pub bias: f64,
    pub spread: f64,
    pub chord: f64,
    pub jack: f64,
    pub stream: f64,
    pub fatigue: f64,
    pub focus_drift: f64,
    pub miss_floor: f64,
    /// Extra miss probability at full chord/jack/stream load.
    pub miss_k: f64,
    pub p_slip: f64,
    pub hold_release_ms: f64,
    pub release_bias: f64,
    /// `too-tight` warning threshold (UR).
    pub ur_min: f64,
}

impl Default for Style {
    fn default() -> Self {
        Self::max()
    }
}

#[allow(clippy::too_many_arguments)]
fn preset(
    name: &str,
    target: f64,
    sigma0: f64,
    bias: f64,
    (chord, jack, stream, fatigue, focus_drift): (f64, f64, f64, f64, f64),
    (miss_floor, miss_k, hold_release_ms): (f64, f64, f64),
    spread: f64,
) -> Style {
    Style {
        name: name.into(),
        target,
        sigma0,
        bias,
        spread,
        chord,
        jack,
        stream,
        fatigue,
        focus_drift,
        miss_floor,
        miss_k,
        p_slip: 0.02,
        hold_release_ms,
        release_bias: bias,
        ur_min: 50.0,
    }
}

impl Style {
    pub fn max() -> Self {
        Self {
            name: "Max".into(),
            target: 100.0,
            sigma0: 0.0,
            bias: 0.0,
            spread: 1.0,
            chord: 0.0,
            jack: 0.0,
            stream: 0.0,
            fatigue: 0.0,
            focus_drift: 0.0,
            miss_floor: 0.0,
            miss_k: 0.0,
            p_slip: 0.0,
            hold_release_ms: 0.0,
            release_bias: 0.0,
            ur_min: 0.0,
        }
    }

    /// Max, Sweaty, Casual friend, Rusty (compiled in).
    pub fn builtins() -> Vec<Style> {
        vec![
            Self::max(),
            preset(
                "Sweaty",
                96.0,
                14.0,
                3.0,
                (0.3, 0.4, 0.3, 0.2, 0.15),
                (0.004, 0.01, 18.0),
                1.0,
            ),
            preset(
                "Casual friend",
                88.0,
                26.0,
                8.0,
                (0.6, 0.8, 0.5, 0.3, 0.3),
                (0.015, 0.03, 30.0),
                1.0,
            ),
            preset(
                "Rusty",
                82.0,
                30.0,
                15.0,
                (0.5, 0.6, 0.6, 0.6, 0.4),
                (0.02, 0.04, 35.0),
                1.1,
            ),
        ]
    }

    pub fn builtin(name: &str) -> Option<Style> {
        Self::builtins()
            .into_iter()
            .find(|s| s.name.eq_ignore_ascii_case(name))
    }

    /// No error, no misses, no drift: the planner skips the model.
    pub fn is_max(&self) -> bool {
        self.sigma0 <= 0.0
            && self.bias == 0.0
            && self.miss_floor <= 0.0
            && self.miss_k <= 0.0
            && self.hold_release_ms <= 0.0
            && self.release_bias == 0.0
    }
}

/// Name for a style whose advanced values were edited from `base`.
pub fn custom_name(base: &str) -> String {
    let base = base
        .strip_prefix("Custom (based on ")
        .and_then(|b| b.strip_suffix(')'))
        .unwrap_or(base);
    format!("Custom (based on {base})")
}

pub fn chord_term(size: usize) -> f64 {
    match size {
        0 | 1 => 0.0,
        2 => 0.5,
        _ => 1.0,
    }
}

pub fn jack_term(gap_ms: f64) -> f64 {
    ((150.0 - gap_ms) / 100.0).clamp(0.0, 1.0)
}

pub fn stream_term(nps: f64) -> f64 {
    ((nps - 6.0) / 10.0).clamp(0.0, 1.0)
}

/// What the model knows about a planned note. `t_ms` is song time.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NoteCtx {
    pub t_ms: f64,
    pub c: f64,
    pub j: f64,
    pub s: f64,
}

/// `(lane, t_ms)` sorted by time -> contexts, using the same terms the planner computes from tracks.
pub fn contexts(chart: &[(u8, f64)]) -> Vec<NoteCtx> {
    chart
        .iter()
        .enumerate()
        .map(|(i, &(lane, t))| {
            let size = chart.iter().filter(|o| (o.1 - t).abs() <= 5.0).count();
            let gap = chart[..i]
                .iter()
                .rev()
                .find(|o| o.0 == lane && t - o.1 > 5.0)
                .map_or(f64::INFINITY, |o| t - o.1);
            let n = chart
                .iter()
                .filter(|o| o.1 > t - 1000.0 && o.1 <= t)
                .count();
            NoteCtx {
                t_ms: t,
                c: chord_term(size),
                j: jack_term(gap),
                s: stream_term(n as f64),
            }
        })
        .collect()
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NoteDraw {
    /// Added to the crossing, ms (late positive).
    pub error_ms: f64,
    /// Added to a hold's end, ms. The planner clamps it to keep 60% of the hold.
    pub release_error_ms: f64,
    /// Not pressed at all.
    pub miss: bool,
}

fn uniform(r: &mut ChaCha8Rng) -> f64 {
    ((r.next_u64() >> 11) + 1) as f64 * (1.0 / 9_007_199_254_740_992.0)
}

fn normal(r: &mut ChaCha8Rng) -> f64 {
    let (a, b) = (uniform(r), uniform(r));
    (-2.0 * a.ln()).sqrt() * (std::f64::consts::TAU * b).cos()
}

/// Standard normal CDF (Abramowitz-Stegun 7.1.26, abs error < 2e-7).
fn phi(x: f64) -> f64 {
    let z = x.abs() / std::f64::consts::SQRT_2;
    let t = 1.0 / (1.0 + 0.327_591_1 * z);
    let poly = t
        * (0.254_829_592
            + t * (-0.284_496_736
                + t * (1.421_413_741 + t * (-1.453_152_027 + t * 1.061_405_429))));
    let erf = 1.0 - poly * (-z * z).exp();
    if x >= 0.0 {
        0.5 * (1.0 + erf)
    } else {
        0.5 * (1.0 - erf)
    }
}

/// Window and weight values as the model needs them.
#[derive(Clone, Copy, Debug)]
pub struct Judge {
    pub windows: [f64; 4],
    pub weights: [f64; 5],
}

impl Judge {
    pub fn from_game(g: &Game) -> Self {
        let (w, a) = (&g.windows_ms, &g.accuracy_weights);
        Self {
            windows: [w.sick, w.good, w.ok, w.bad].map(f64::from),
            weights: [a.sick, a.good, a.ok, a.bad, a.miss].map(f64::from),
        }
    }

    /// Judgment index 0..=4 (Sick..Miss) of a hit error.
    pub fn classify(&self, err_ms: f64) -> usize {
        self.windows
            .iter()
            .position(|&w| err_ms.abs() <= w)
            .unwrap_or(4)
    }

    /// Probability of each judgment for a note with error spread `sigma` around `mu` (drift sd
    /// `drift_sd` adds in quadrature), before misses (mixture with slips).
    pub fn probs(&self, mu: f64, sigma: f64, drift_sd: f64, p_slip: f64) -> [f64; 5] {
        let s = sigma.max(1e-6);
        let within = |w: f64| {
            let one = |s: f64| {
                let s = (s * s + drift_sd * drift_sd).sqrt();
                phi((w - mu) / s) - phi((-w - mu) / s)
            };
            (1.0 - p_slip) * one(s) + p_slip * one(3.0 * s)
        };
        let mut p = [0.0; 5];
        let mut prev = 0.0;
        for (i, &w) in self.windows.iter().enumerate() {
            let c = within(w);
            p[i] = c - prev;
            prev = c;
        }
        p[4] = 1.0 - prev;
        p
    }

    /// Expected accuracy (0..100) of notes with spread multiplier `mult` and miss
    /// probability `miss_p` at gain `k`.
    pub fn expected_accuracy(&self, st: &Style, k: f64, mult: f64, miss_p: f64) -> f64 {
        let p = self.probs(
            st.bias,
            st.sigma0 * k * st.spread * mult,
            st.focus_drift * DRIFT_SD_MS,
            st.p_slip,
        );
        let acc: f64 = p.iter().zip(self.weights).map(|(p, w)| p * w).sum();
        100.0 * ((1.0 - miss_p) * acc + miss_p * self.weights[4])
    }

    /// Feed-forward gain for `target` (bisection, k in [0.2, 4]).
    pub fn solve_k(&self, st: &Style, target: f64, mult: f64, miss_p: f64) -> f64 {
        let f = |k| self.expected_accuracy(st, k, mult, miss_p) - target;
        if f(K_MIN) <= 0.0 {
            return K_MIN;
        }
        if f(K_MAX) >= 0.0 {
            return K_MAX;
        }
        let (mut lo, mut hi) = (K_MIN, K_MAX);
        for _ in 0..40 {
            let mid = 0.5 * (lo + hi);
            if f(mid) > 0.0 { lo = mid } else { hi = mid }
        }
        0.5 * (lo + hi)
    }
}

/// Self-check numbers for the Live and History pages.
#[derive(Clone, Debug, PartialEq)]
pub struct SelfCheck {
    pub notes: u32,
    pub mean_ms: f64,
    /// sd(hit error) x 10.
    pub ur: f64,
    pub hist: [u32; HIST_BINS],
    pub uniform_like: bool,
    pub too_tight: bool,
}

/// Expected result of a style before a song (Style page).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Preview {
    pub k: f64,
    /// Spread of the hit error, ms (model sigma at gain `k`, slips and drift included).
    pub sigma_ms: f64,
    pub mean_ms: f64,
    /// Sick, Good, Ok, Bad, Miss.
    pub probs: [f64; 5],
    /// Feed-forward accuracy, percent (rough: real accuracy is not count-weighted, G2).
    pub accuracy: f64,
}

/// Model, controller and self-check in one preallocated object (no allocation per note).
pub struct StylePlayer {
    pub style: Style,
    pub judge: Judge,
    seed: u64,
    max: bool,
    next_index: u64,
    t0: Option<f64>,
    last_t: f64,
    drift: f64,
    // Controller.
    ctl_on: bool,
    k: f64,
    u0: f64,
    integral: f64,
    last_update_s: f64,
    total_notes: u32,
    hud_acc: Option<f64>,
    // Running estimates of what the chart looks like.
    mult_sum: f64,
    load_sum: f64,
    // Played notes, their realised weight, and error stats.
    played: u32,
    weight_sum: f64,
    n_err: u32,
    mean: f64,
    m2: f64,
    hist: [u32; HIST_BINS],
    ring: Vec<f64>,
    ring_pos: usize,
}

impl StylePlayer {
    /// `expected_notes`: song length guess for the controller (replace with `set_total_notes`).
    pub fn new(style: Style, game: &Game, seed: u64, expected_notes: u32) -> Self {
        let max = style.is_max();
        let judge = Judge::from_game(game);
        let mut p = Self {
            max,
            judge,
            seed,
            next_index: 0,
            t0: None,
            last_t: 0.0,
            drift: 0.0,
            ctl_on: !max,
            k: 1.0,
            u0: 0.0,
            integral: 0.0,
            last_update_s: 0.0,
            total_notes: expected_notes.max(1),
            hud_acc: None,
            mult_sum: 0.0,
            load_sum: 0.0,
            played: 0,
            weight_sum: 0.0,
            n_err: 0,
            mean: 0.0,
            m2: 0.0,
            hist: [0; HIST_BINS],
            ring: Vec::with_capacity(RING),
            ring_pos: 0,
            style,
        };
        if !max {
            let m = p.mult_estimate();
            p.k = judge.solve_k(&p.style, p.style.target, m, p.miss_estimate());
            p.u0 = p.k.ln();
        }
        p
    }

    /// Fixed gain, controller off (model statistics tests).
    pub fn with_fixed_k(mut self, k: f64) -> Self {
        self.k = k;
        self.ctl_on = false;
        self
    }

    pub fn is_max(&self) -> bool {
        self.max
    }

    pub fn k(&self) -> f64 {
        self.k
    }

    pub fn set_total_notes(&mut self, n: u32) {
        self.total_notes = n.max(1);
    }

    /// Accuracy shown by the game so far (percent), or `None` to fall back to our own estimate.
    pub fn feed_accuracy(&mut self, measured: Option<f64>) {
        self.hud_acc = measured;
    }

    /// Our estimate of accuracy so far from planned errors (percent).
    pub fn own_accuracy(&self) -> f64 {
        if self.played == 0 {
            self.style.target
        } else {
            100.0 * self.weight_sum / self.played as f64
        }
    }

    pub fn played(&self) -> u32 {
        self.played
    }

    fn mult_estimate(&self) -> f64 {
        let st = &self.style;
        if self.played == 0 {
            (1.0 + 0.2 * (st.chord + st.jack + st.stream)) * (1.0 + 0.5 * st.fatigue)
        } else {
            self.mult_sum / self.played as f64
        }
    }

    fn miss_estimate(&self) -> f64 {
        let load = if self.played == 0 {
            0.15
        } else {
            self.load_sum / self.played as f64
        };
        (self.style.miss_floor + self.style.miss_k * load).clamp(0.0, 1.0)
    }

    /// Draws the error for the next planned note and steps the controller. Deterministic in
    /// `(seed, note order)`.
    pub fn note(&mut self, ctx: NoteCtx) -> NoteDraw {
        if self.max {
            return NoteDraw::default();
        }
        let st = &self.style;
        let idx = self.next_index;
        self.next_index += 1;
        let t0 = *self.t0.get_or_insert(ctx.t_ms);
        let t = (ctx.t_ms - t0).max(0.0);
        let mut seedb = [0u8; 32];
        seedb[..8].copy_from_slice(&self.seed.to_le_bytes());
        seedb[8..16].copy_from_slice(&idx.to_le_bytes());
        let mut r = ChaCha8Rng::from_seed(seedb);

        // Ornstein-Uhlenbeck drift, exact discretisation.
        let sd = st.focus_drift * DRIFT_SD_MS;
        let z = normal(&mut r);
        if idx == 0 {
            self.drift = sd * z;
        } else {
            let a = (-(t - self.last_t).max(0.0) / DRIFT_TAU_MS).exp();
            self.drift = self.drift * a + sd * (1.0 - a * a).sqrt() * z;
        }
        self.last_t = t.max(self.last_t);

        let load = 1.0 + st.chord * ctx.c + st.jack * ctx.j + st.stream * ctx.s;
        let mult = load * (1.0 + st.fatigue * t / T_REF_MS);
        let sigma = st.sigma0 * self.k * st.spread * mult;
        let slip = uniform(&mut r) < st.p_slip;
        let e = st.bias + self.drift + sigma * if slip { 3.0 } else { 1.0 } * normal(&mut r);
        let miss_p = st.miss_floor + st.miss_k * (ctx.c + ctx.j + ctx.s) / 3.0;
        let miss = uniform(&mut r) < miss_p;
        let rel = st.release_bias + st.hold_release_ms * normal(&mut r);

        self.mult_sum += mult;
        self.load_sum += (ctx.c + ctx.j + ctx.s) / 3.0;
        self.played += 1;
        if miss {
            self.weight_sum += self.judge.weights[4];
        } else {
            self.weight_sum += self.judge.weights[self.judge.classify(e)];
            self.record(e);
        }
        if self.ctl_on {
            self.update(t / 1000.0);
        }
        NoteDraw {
            error_ms: if miss { 0.0 } else { e },
            release_error_ms: rel,
            miss,
        }
    }

    fn record(&mut self, e: f64) {
        self.n_err += 1;
        let d = e - self.mean;
        self.mean += d / self.n_err as f64;
        self.m2 += d * (e - self.mean);
        let bin = (e / HIST_BIN_MS).round() as i64 + (HIST_BINS / 2) as i64;
        self.hist[bin.clamp(0, HIST_BINS as i64 - 1) as usize] += 1;
        if self.ring.len() < RING {
            self.ring.push(e);
        } else {
            self.ring[self.ring_pos] = e;
            self.ring_pos = (self.ring_pos + 1) % RING;
        }
    }

    /// PI on log k, at most every 2 s of song time, at most 10% per update.
    fn update(&mut self, now_s: f64) {
        if now_s - self.last_update_s < UPDATE_EVERY_S {
            return;
        }
        let dt = now_s - self.last_update_s;
        self.last_update_s = now_s;
        let total = self.total_notes.max(self.played) as f64;
        let played = self.played as f64;
        let measured = self.hud_acc.unwrap_or_else(|| self.own_accuracy());
        let ff = self.judge.expected_accuracy(
            &self.style,
            self.k,
            self.mult_estimate(),
            self.miss_estimate(),
        );
        let projected = (played * measured + (total - played) * ff) / total;
        // Error in accuracy points; positive means we are heading below target.
        let e = self.style.target - projected;
        let lim = (K_MAX / K_MIN).ln() / KI;
        self.integral = (self.integral + e * dt).clamp(-lim, lim);
        let u_want = self.u0 - (KP * e + KI * self.integral);
        let u = self.k.ln();
        let u_new = u_want.clamp(u - MAX_STEP, u + MAX_STEP);
        self.k = u_new.exp().clamp(K_MIN, K_MAX);
    }

    /// What the Style page shows before a song: the feed-forward gain for the target and the
    /// judgment split, spread and mean it expects on an average chart.
    pub fn preview(&self) -> Preview {
        let st = &self.style;
        if self.max {
            return Preview {
                k: 1.0,
                sigma_ms: 0.0,
                mean_ms: 0.0,
                probs: [1.0, 0.0, 0.0, 0.0, 0.0],
                accuracy: 100.0,
            };
        }
        let (mult, miss) = (self.mult_estimate(), self.miss_estimate());
        let sigma = st.sigma0 * self.k * st.spread * mult;
        let drift_sd = st.focus_drift * DRIFT_SD_MS;
        let mut probs = self.judge.probs(st.bias, sigma, drift_sd, st.p_slip);
        for p in &mut probs {
            *p *= 1.0 - miss;
        }
        probs[4] += miss;
        Preview {
            k: self.k,
            // The spread a hit error shows: slips widen it, drift adds in quadrature.
            sigma_ms: (sigma * sigma * (1.0 + 8.0 * st.p_slip) + drift_sd * drift_sd).sqrt(),
            mean_ms: st.bias,
            probs,
            accuracy: self.judge.expected_accuracy(st, self.k, mult, miss),
        }
    }

    pub fn stats(&self) -> SelfCheck {
        let n = self.n_err;
        let ur = if n > 1 {
            (self.m2 / (n - 1) as f64).sqrt() * 10.0
        } else {
            0.0
        };
        SelfCheck {
            notes: n,
            mean_ms: self.mean,
            ur,
            hist: self.hist,
            uniform_like: n >= 200 && ks_uniform_p(&self.ring) > 0.05,
            too_tight: !self.max && n >= 30 && ur < self.style.ur_min,
        }
    }
}

/// p-value of a KS test of `xs` against a uniform on its observed range.
fn ks_uniform_p(xs: &[f64]) -> f64 {
    let mut v = xs.to_vec();
    v.sort_by(f64::total_cmp);
    let (lo, hi) = (v[0], v[v.len() - 1]);
    if hi <= lo {
        return 1.0;
    }
    let n = v.len() as f64;
    let d = v
        .iter()
        .enumerate()
        .map(|(i, &x)| {
            let f = (x - lo) / (hi - lo);
            ((i as f64 + 1.0) / n - f)
                .abs()
                .max((f - i as f64 / n).abs())
        })
        .fold(0.0, f64::max);
    let l = (n.sqrt() + 0.12 + 0.11 / n.sqrt()) * d;
    let mut p = 0.0;
    for j in 1..=100 {
        let j = j as f64;
        let term = 2.0 * (-2.0 * j * j * l * l).exp();
        p += if (j as i64) % 2 == 1 { term } else { -term };
        if term < 1e-12 {
            break;
        }
    }
    p.clamp(0.0, 1.0)
}

// ---- Files: `<config_dir>/styles/*.toml` ----

#[derive(Debug)]
pub enum StyleError {
    /// Name matches a compiled-in preset.
    BuiltIn,
    /// Empty, or contains a path separator or dot-dot.
    BadName,
    Config(ConfigError),
}

impl std::fmt::Display for StyleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BuiltIn => write!(f, "built-in styles cannot be overwritten"),
            Self::BadName => write!(f, "invalid style name"),
            Self::Config(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for StyleError {}

impl From<ConfigError> for StyleError {
    fn from(e: ConfigError) -> Self {
        Self::Config(e)
    }
}

fn style_path(config_dir: &Path, name: &str) -> Result<PathBuf, StyleError> {
    if name.trim().is_empty() || name.contains(['/', '\\', '\0']) || name.contains("..") {
        return Err(StyleError::BadName);
    }
    Ok(config_dir.join("styles").join(format!("{name}.toml")))
}

/// Saves a user style to `styles/<name>.toml`, atomically.
pub fn save_style(config_dir: &Path, style: &Style) -> Result<(), StyleError> {
    if Style::builtin(&style.name).is_some() {
        return Err(StyleError::BuiltIn);
    }
    Ok(save_toml(&style_path(config_dir, &style.name)?, style)?)
}

pub fn delete_style(config_dir: &Path, name: &str) -> Result<(), StyleError> {
    if Style::builtin(name).is_some() {
        return Err(StyleError::BuiltIn);
    }
    fs::remove_file(style_path(config_dir, name)?).map_err(|e| ConfigError::Io(e).into())
}

/// Built-ins first, then user styles sorted by name. Unreadable files are skipped; a user
/// file named like a built-in is ignored.
pub fn list_styles(config_dir: &Path) -> Vec<Style> {
    let mut out = Style::builtins();
    let mut user = Vec::new();
    if let Ok(rd) = fs::read_dir(config_dir.join("styles")) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_none_or(|x| x != "toml") {
                continue;
            }
            let Some(stem) = p.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if Style::builtin(stem).is_some() {
                continue;
            }
            if let Ok(mut s) = load_toml::<Style>(&p) {
                s.name = stem.to_string();
                user.push(s);
            }
        }
    }
    user.sort_by(|a, b| a.name.cmp(&b.name));
    out.extend(user);
    out
}

/// Active style by name (`config.toml` `style`); unknown names fall back to Max.
pub fn find_style(config_dir: Option<&Path>, name: &str) -> Style {
    if let Some(s) = Style::builtin(name) {
        return s;
    }
    config_dir
        .and_then(|d| list_styles(d).into_iter().find(|s| s.name == name))
        .unwrap_or_else(Style::max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_hits_target_and_sums_to_one() {
        let g = Game::default();
        for st in Style::builtins() {
            let p = StylePlayer::new(st.clone(), &g, 1, 600).preview();
            assert!(
                (p.probs.iter().sum::<f64>() - 1.0).abs() < 1e-9,
                "{}",
                st.name
            );
            assert!((p.accuracy - st.target).abs() < 0.05, "{}: {p:?}", st.name);
        }
    }

    #[test]
    fn ks_separates_uniform_from_normal() {
        let mut r = ChaCha8Rng::seed_from_u64(1);
        let uni: Vec<f64> = (0..500).map(|_| uniform(&mut r) * 80.0 - 40.0).collect();
        let nor: Vec<f64> = (0..500).map(|_| normal(&mut r) * 20.0).collect();
        assert!(ks_uniform_p(&uni) > 0.05);
        assert!(ks_uniform_p(&nor) < 0.05);
    }
}
