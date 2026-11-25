//! The opponent (spec 06, match my friend): which score is ours, and a target that follows
//! theirs.

use crate::config::Seat;
use crate::frame::Rect;

/// Our score's index in `[left, right]`. `Auto`: the side the stats panel is on (the game
/// puts our stats, counts and score on our side); `None` until the panel is found.
pub fn our_side(seat: Seat, stats: Option<Rect>, stream_w: u32) -> Option<usize> {
    match seat {
        Seat::Left => Some(0),
        Seat::Right => Some(1),
        Seat::Auto => stats.map(|r| usize::from(r.x + r.w as i32 / 2 >= stream_w as i32 / 2)),
    }
}

/// `(ours, theirs)` from scores by screen side.
pub fn ours_theirs<T: Copy>(scores: [T; 2], ours: usize) -> (T, T) {
    (scores[ours & 1], scores[(ours & 1) ^ 1])
}

/// Below this, their score says little (song start); the target holds.
const MIN_SCORE: f64 = 2000.0;
/// Close the score gap over about this many updates.
const CLOSE_IN: f64 = 20.0;
/// Target points per unit of relative rate error, per update.
const GAIN: f64 = 4.0;
/// Biggest step, target points per update.
const MAX_STEP: f64 = 1.0;

/// Moves the accuracy target until our score lands `margin_pct` percent from theirs. The game's
/// score formula is unknown and scores add up over the song, so it steers the gap's rate: the
/// gap should close over `CLOSE_IN` updates, and the target moves by how far our scoring rate
/// is off that, relative to theirs. Call it about once a second with fresh scores.
#[derive(Clone, Debug)]
pub struct Matcher {
    pub margin_pct: f64,
    pub target: f64,
    last: Option<(f64, f64)>,
}

impl Matcher {
    pub fn new(margin_pct: f64, target: f64) -> Self {
        Self {
            margin_pct: margin_pct.clamp(-3.0, 3.0),
            target: target.clamp(70.0, 100.0),
            last: None,
        }
    }

    /// The new target, or `None` (hold) while their score is too small or not moving.
    pub fn update(&mut self, ours: i32, theirs: i32) -> Option<f64> {
        let aim = f64::from(theirs) * (1.0 + self.margin_pct / 100.0);
        let gap = aim - f64::from(ours);
        let last = self.last.replace((aim, gap));
        let (last_aim, last_gap) = last?;
        let their_rate = aim - last_aim;
        if aim < MIN_SCORE || their_rate <= 0.0 {
            return None;
        }
        // the gap should shrink by gap / CLOSE_IN this update; it changed by (gap - last_gap)
        let off = (gap - last_gap + last_gap / CLOSE_IN) / their_rate;
        self.target = (self.target + (GAIN * off).clamp(-MAX_STEP, MAX_STEP)).clamp(70.0, 100.0);
        Some(self.target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn side_from_stats_panel_or_config() {
        let left = Some(Rect::new(6, 600, 260, 280));
        let right = Some(Rect::new(2290, 600, 260, 280));
        assert_eq!(our_side(Seat::Auto, left, 2560), Some(0));
        assert_eq!(our_side(Seat::Auto, right, 2560), Some(1));
        assert_eq!(our_side(Seat::Auto, None, 2560), None);
        assert_eq!(our_side(Seat::Right, left, 2560), Some(1));
        assert_eq!(ours_theirs([-395, 63082], 0), (-395, 63082));
        assert_eq!(ours_theirs([-395, 63082], 1), (63082, -395));
    }

    /// A made-up score model (points per second rise steeply with accuracy) and a friend at
    /// 92%: after two minutes our score sits within 1.5% of theirs plus the margin.
    #[test]
    fn target_follows_their_score() {
        let rate = |acc: f64| 400.0 * (acc / 100.0).powi(6);
        for margin in [-3.0, 0.0, 3.0] {
            let mut m = Matcher::new(margin, 99.0);
            let (mut ours, mut theirs) = (0.0, 0.0);
            for _ in 0..240 {
                ours += rate(m.target);
                theirs += rate(92.0);
                m.update(ours as i32, theirs as i32);
            }
            let aim = theirs * (1.0 + margin / 100.0);
            assert!(
                (ours - aim).abs() / aim < 0.015,
                "margin {margin}: {ours} vs {aim}"
            );
        }
    }

    /// Their accuracy drops mid-song and reads come 1-3 s apart: still within 2% at the end.
    #[test]
    fn follows_a_change_with_uneven_reads() {
        let rate = |acc: f64| 400.0 * (acc / 100.0).powi(6);
        let mut m = Matcher::new(0.0, 90.0);
        let (mut ours, mut theirs) = (0.0, 0.0);
        for t in 0..300 {
            ours += rate(m.target);
            theirs += rate(if t < 120 { 96.0 } else { 85.0 });
            if t % 3 != 1 {
                m.update(ours as i32, theirs as i32);
            }
        }
        assert!((ours - theirs).abs() / theirs < 0.02, "{ours} vs {theirs}");
    }

    #[test]
    fn holds_early_and_clamps() {
        let mut m = Matcher::new(0.0, 95.0);
        assert_eq!(m.update(0, 0), None);
        assert_eq!(m.update(0, 500), None);
        assert_eq!(m.target, 95.0);
        for i in 0..100 {
            m.update(-5000, 50_000 + i * 100);
        }
        assert_eq!(m.target, 100.0);
    }
}
