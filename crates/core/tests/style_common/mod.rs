//! Shared helpers for the style tests.
#![allow(dead_code)]

use arrowhead_core::config::Game;
use arrowhead_core::style::{Style, StylePlayer, contexts};

pub struct Lcg(pub u64);
impl Lcg {
    pub fn next(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// `(lane, t_ms)`: gaps 120-420 ms, 10% two-note chords, 3% fast jacks.
pub fn chart(seed: u64, n: usize) -> Vec<(u8, f64)> {
    let mut r = Lcg(seed.wrapping_mul(0x9E37_79B9).wrapping_add(7));
    let (mut t, mut out) = (1000.0, Vec::with_capacity(n + 1));
    while out.len() < n {
        let lane = (r.next() * 4.0) as u8 % 4;
        out.push((lane, t));
        if r.next() < 0.10 {
            out.push(((lane + 1 + (r.next() * 3.0) as u8 % 3) % 4, t));
        }
        t += if r.next() < 0.03 {
            60.0
        } else {
            120.0 + r.next() * 300.0
        };
    }
    out.truncate(n);
    out
}

pub fn game() -> Game {
    Game::default()
}

pub fn presets() -> Vec<Style> {
    Style::builtins()
        .into_iter()
        .filter(|s| !s.is_max())
        .collect()
}

/// Plays `n` notes; returns the player, the errors of pressed notes and every judgment (0-4).
pub fn run(
    style: &Style,
    seed: u64,
    n: usize,
    k: Option<f64>,
) -> (StylePlayer, Vec<f64>, Vec<usize>) {
    let ctx = contexts(&chart(seed, n));
    let mut p = StylePlayer::new(style.clone(), &game(), seed, n as u32);
    if let Some(k) = k {
        p = p.with_fixed_k(k);
    }
    let (mut errs, mut judg) = (Vec::new(), Vec::new());
    for c in ctx {
        let d = p.note(c);
        if d.miss {
            judg.push(4);
        } else {
            errs.push(d.error_ms);
            judg.push(p.judge.classify(d.error_ms));
        }
    }
    (p, errs, judg)
}
