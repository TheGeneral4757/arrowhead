//! Readback (spec 04): match each new ms readout to the press that caused it and
//! nudge `game_bias`. Pure; the runner feeds it reads and recent presses.

use crate::time::Nanos;

/// A committed press, as sent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SentPress {
    pub sent: Nanos,
    pub lane: u8,
    /// Error the style model intended, ms, positive = late. Max style: 0.
    pub intended_err_ms: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Matched {
    /// Index into the `presses` slice passed to `on_read`.
    pub press: usize,
    /// Hit error normalised so positive = late.
    pub hit_error_ms: f32,
    pub intended_err_ms: f32,
    /// `game_bias` after this read.
    pub bias_ms: f32,
}

#[derive(Clone, Debug)]
pub struct Readback {
    /// -1: the game shows late hits as negative ms (G4).
    pub sign: f32,
    pub gain: f32,
    pub clamp_ms: f32,
    pub min_age_ms: f64,
    pub max_age_ms: f64,
    pub bias_ms: f32,
    last: Option<f32>,
    blank: bool,
}

impl Readback {
    pub fn new(bias_ms: f32) -> Self {
        Self {
            sign: -1.0,
            gain: 0.05,
            clamp_ms: 60.0,
            min_age_ms: 30.0,
            max_age_ms: 400.0,
            bias_ms,
            last: None,
            blank: true,
        }
    }

    /// The ms region held no text this tick: the next read is a new readout even if equal.
    pub fn on_blank(&mut self) {
        self.blank = true;
    }

    /// One HUD tick with a successful ms read taken at `at` (frame pts).
    /// `lane` is the judgment popup's lane when known. Returns `None` when the readout is
    /// unchanged since the last tick, or no press was sent 30-400 ms before it
    /// (unmatched reads are dropped).
    pub fn on_read(
        &mut self,
        at: Nanos,
        raw_ms: f32,
        lane: Option<u8>,
        presses: &[SentPress],
    ) -> Option<Matched> {
        let new = self.blank || self.last != Some(raw_ms);
        self.last = Some(raw_ms);
        self.blank = false;
        if !new {
            return None;
        }
        let age = |p: &SentPress| at.0 as f64 / 1e6 - p.sent.0 as f64 / 1e6;
        let i = presses
            .iter()
            .enumerate()
            .filter(|(_, p)| {
                (self.min_age_ms..=self.max_age_ms).contains(&age(p))
                    && lane.is_none_or(|l| l == p.lane)
            })
            .min_by(|a, b| age(a.1).total_cmp(&age(b.1)))?
            .0;
        let hit = raw_ms * self.sign;
        let want = presses[i].intended_err_ms;
        self.bias_ms =
            (self.bias_ms - self.gain * (hit - want)).clamp(-self.clamp_ms, self.clamp_ms);
        Some(Matched {
            press: i,
            hit_error_ms: hit,
            intended_err_ms: want,
            bias_ms: self.bias_ms,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(ms: f64, lane: u8, want: f32) -> SentPress {
        SentPress {
            sent: Nanos::from_ms(ms),
            lane,
            intended_err_ms: want,
        }
    }

    #[test]
    fn matches_latest_in_window_and_moves_bias() {
        let mut rb = Readback::new(-5.0);
        let ps = [
            press(0.0, 0, 0.0),
            press(100.0, 1, 0.0),
            press(480.0, 2, 0.0),
        ];
        // read at 500: lane 2 is only 20 ms old (too fresh), lane 1 is 400 old (edge), lane 0 too old
        let m = rb.on_read(Nanos::from_ms(500.0), -10.0, None, &ps).unwrap();
        assert_eq!(m.press, 1);
        assert_eq!(m.hit_error_ms, 10.0); // late
        assert!((m.bias_ms - (-5.5)).abs() < 1e-5); // late -> bias down
    }

    #[test]
    fn unchanged_read_is_not_new_until_blank() {
        let mut rb = Readback::new(0.0);
        let ps = [press(0.0, 0, 0.0)];
        assert!(rb.on_read(Nanos::from_ms(100.0), 4.0, None, &ps).is_some());
        assert!(rb.on_read(Nanos::from_ms(150.0), 4.0, None, &ps).is_none());
        rb.on_blank();
        assert!(rb.on_read(Nanos::from_ms(200.0), 4.0, None, &ps).is_some());
    }

    #[test]
    fn lane_filter_intended_error_and_clamp() {
        let mut rb = Readback::new(59.9);
        let ps = [press(0.0, 3, 20.0), press(50.0, 1, 0.0)];
        assert!(
            rb.on_read(Nanos::from_ms(200.0), 0.0, Some(2), &ps)
                .is_none()
        );
        let m = rb
            .on_read(Nanos::from_ms(200.0), 8.0, Some(3), &ps)
            .unwrap();
        // hit = -8 (early), intended +20: bias -= 0.05 * -28 = +1.4, clamped at 60
        assert_eq!(m.press, 0);
        assert_eq!(m.bias_ms, 60.0);
        let mut rb = Readback::new(0.0);
        assert!(rb.on_read(Nanos::from_ms(10.0), 1.0, None, &ps).is_none()); // too fresh
    }
}
