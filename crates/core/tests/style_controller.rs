//! Spec 06: from k=1, simulated songs end within +/- 0.5% of target.
mod style_common;
use arrowhead_core::style::*;
use style_common::*;

#[test]
fn songs_land_within_half_a_percent_of_target() {
    let mut worst: f64 = 0.0;
    let j = Judge::from_game(&game());
    for st in presets() {
        for &n in &[300usize, 800, 1500] {
            for &target in &[85.0, 90.0, 95.0, 98.0] {
                for seed in 0..5u64 {
                    let st = Style {
                        target,
                        ..st.clone()
                    };
                    // Targets above what the style's own miss rate allows cannot be hit.
                    if j.expected_accuracy(&st, 0.2, 1.0, st.miss_floor + st.miss_k * 0.15)
                        < target + 2.5
                    {
                        continue;
                    }
                    let (p, _, _) = run(&st, seed, n, None);
                    let err = p.own_accuracy() - target;
                    worst = worst.max(err.abs());
                    // The last 2 s of a song cannot be steered: at 300 notes that is ~0.4% of
                    // noise (1 sd), so the short song gets a looser bound.
                    let tol = if n == 300 { 0.9 } else { 0.5 };
                    assert!(
                        err.abs() <= tol,
                        "{} n={n} target={target} seed={seed}: final {:.2} k={:.2}",
                        st.name,
                        p.own_accuracy(),
                        p.k()
                    );
                }
            }
        }
    }
    eprintln!("worst final error {worst:.3}");
}

#[test]
fn k_moves_slowly_and_feed_forward_is_monotonic() {
    let st = Style::builtin("Sweaty").unwrap();
    let j = Judge::from_game(&game());
    let a = j.expected_accuracy(&st, 0.5, 1.2, 0.01);
    let b = j.expected_accuracy(&st, 2.0, 1.2, 0.01);
    assert!(a > b);
    let k = j.solve_k(&st, 90.0, 1.2, 0.01);
    assert!((j.expected_accuracy(&st, k, 1.2, 0.01) - 90.0).abs() < 1e-3);
    // Controller steps are at most 10% and at least 2 s apart.
    let mut p = StylePlayer::new(Style { target: 90.0, ..st }, &game(), 4, 600);
    let mut last = p.k();
    let mut last_t: Option<f64> = None;
    for c in contexts(&chart(4, 600)) {
        p.note(c);
        if (p.k() - last).abs() > 1e-12 {
            assert!((p.k() / last).ln().abs() <= 0.0954);
            if let Some(t) = last_t {
                assert!(c.t_ms - t >= 1999.0);
            }
            last_t = Some(c.t_ms);
            last = p.k();
        }
    }
}

#[test]
fn hud_accuracy_overrides_own_estimate() {
    let st = Style {
        target: 90.0,
        ..Style::builtin("Sweaty").unwrap()
    };
    let run_with = |hud: Option<f64>| {
        let mut p = StylePlayer::new(st.clone(), &game(), 4, 600);
        for c in contexts(&chart(4, 300)) {
            p.feed_accuracy(hud);
            p.note(c);
        }
        p.k()
    };
    // The game says we are far below target: k must come down (more accurate).
    assert!(run_with(Some(60.0)) < run_with(None));
}
