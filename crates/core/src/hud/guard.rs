//! Atlas consistency guard (spec 13, 1.2): the stats panel and the counts must agree. Hits is
//! Sick+Good+Ok+Bad, the two Miss counters are equal, and no counter goes down in a song. A
//! mis-taught digit breaks one of these on every read, so a contradiction that holds on
//! several stable reads (the same numbers two ticks running, so not a panel mid-update)
//! condemns the atlas.

use super::{Counts, StatsRead};

/// Stable contradicting reads before the atlas is blamed.
const STREAK: u8 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Not enough read, or the numbers are still moving.
    Unknown,
    /// Every check that could be made passed.
    Good,
    /// Contradicted on `STREAK` stable reads: roll back the last teach or relearn the face.
    Bad,
}

#[derive(Clone, Debug, Default)]
pub struct Guard {
    last: Option<(StatsRead, Counts)>,
    trusted: Option<[u32; 6]>,
    streak: u8,
}

/// [hits, misses, sick, good, ok, bad] summed rows; `None` entries skip that check.
fn rows(st: &StatsRead, c: &Counts) -> [Option<u32>; 7] {
    [st.hits, st.misses, c.sick, c.good, c.ok, c.bad, c.miss]
}

/// Whether the two panels contradict each other or the last trusted read. `None`: nothing
/// could be checked.
pub fn contradicts(st: &StatsRead, c: &Counts, trusted: Option<&[u32; 6]>) -> Option<bool> {
    let mut checked = false;
    let mut bad = false;
    if let (Some(h), Some(s), Some(g), Some(o), Some(b)) = (st.hits, c.sick, c.good, c.ok, c.bad) {
        checked = true;
        bad |= h != s + g + o + b;
    }
    if let (Some(a), Some(b)) = (st.misses, c.miss) {
        checked = true;
        bad |= a != b;
    }
    if let Some(t) = trusted {
        let now = rows(st, c);
        // hits, misses, sick, good, ok, bad (Miss is checked against Misses above)
        for (i, v) in now[..6].iter().enumerate() {
            if let Some(v) = v {
                checked = true;
                bad |= *v < t[i];
            }
        }
    }
    checked.then_some(bad)
}

impl Guard {
    /// A song started: counters are back at zero.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn check(&mut self, st: &StatsRead, c: &Counts) -> Verdict {
        let stable = self.last == Some((*st, *c));
        self.last = Some((*st, *c));
        if !stable {
            return Verdict::Unknown;
        }
        match contradicts(st, c, self.trusted.as_ref()) {
            None => Verdict::Unknown,
            Some(false) => {
                self.streak = 0;
                // keep the highest trusted value per row (a row that did not read keeps its last)
                let now = rows(st, c);
                let t = self.trusted.get_or_insert([0; 6]);
                for i in 0..6 {
                    if let Some(v) = now[i] {
                        t[i] = v;
                    }
                }
                Verdict::Good
            }
            Some(true) => {
                self.streak += 1;
                if self.streak >= STREAK {
                    self.streak = 0;
                    self.last = None;
                    Verdict::Bad
                } else {
                    Verdict::Unknown
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(hits: u32, misses: u32, c: [u32; 5]) -> (StatsRead, Counts) {
        (
            StatsRead {
                hits: Some(hits),
                misses: Some(misses),
                ..Default::default()
            },
            Counts {
                sick: Some(c[0]),
                good: Some(c[1]),
                ok: Some(c[2]),
                bad: Some(c[3]),
                miss: Some(c[4]),
            },
        )
    }

    #[test]
    fn panels_that_agree_are_good() {
        let mut g = Guard::default();
        let (s, c) = read(153, 28, [150, 2, 1, 0, 28]);
        assert_eq!(g.check(&s, &c), Verdict::Unknown); // first read is not stable yet
        assert_eq!(g.check(&s, &c), Verdict::Good);
    }

    #[test]
    fn mis_taught_digit_is_bad_after_three_stable_reads() {
        // '5' taught as '3': Hits 153 reads 133, counts still add to 153
        let mut g = Guard::default();
        let (s, c) = read(133, 28, [150, 2, 1, 0, 28]);
        let v: Vec<Verdict> = (0..4).map(|_| g.check(&s, &c)).collect();
        assert_eq!(
            v,
            [
                Verdict::Unknown,
                Verdict::Unknown,
                Verdict::Unknown,
                Verdict::Bad
            ]
        );
    }

    #[test]
    fn moving_numbers_are_not_judged() {
        let mut g = Guard::default();
        for i in 0..10 {
            let (s, c) = read(i * 7, 0, [i, 0, 0, 0, 0]);
            assert_eq!(g.check(&s, &c), Verdict::Unknown);
        }
    }

    #[test]
    fn misses_differ_or_counter_goes_down() {
        let mut g = Guard::default();
        let (s, c) = read(10, 2, [10, 0, 0, 0, 3]);
        assert_eq!(contradicts(&s, &c, None), Some(true));
        let (s, c) = read(10, 2, [10, 0, 0, 0, 2]);
        g.check(&s, &c);
        assert_eq!(g.check(&s, &c), Verdict::Good);
        let (s, c) = read(8, 2, [8, 0, 0, 0, 2]);
        assert_eq!(contradicts(&s, &c, g.trusted.as_ref()), Some(true));
        assert_eq!(
            contradicts(&StatsRead::default(), &Counts::default(), None),
            None
        );
    }
}
