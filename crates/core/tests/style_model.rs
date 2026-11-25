//! Spec 06 "Statistical tests": model statistics, determinism, Max bypass, files.
mod style_common;
use arrowhead_core::style::*;
use style_common::*;

const SEEDS: u64 = 50;
const N: usize = 1000;

#[test]
fn mean_ur_and_judgments_match_the_model() {
    for st in presets() {
        let (mut sum, mut cnt) = (0.0, 0usize);
        let mut got = [0.0f64; 5];
        let mut exp = [0.0f64; 5];
        let mut var_exp = [0.0f64; 5];
        for seed in 0..SEEDS {
            let (p, errs, judg) = run(&st, seed, N, Some(1.0));
            sum += errs.iter().sum::<f64>();
            cnt += errs.len();
            for j in &judg {
                got[*j] += 1.0;
            }
            // UR within 10% of the model: E[sigma^2 (1 + 8 p_slip)] + drift variance.
            let ch = chart(seed, N);
            let t0 = ch[0].1;
            let drift_var = (st.focus_drift * 20.0).powi(2);
            let mut v = 0.0;
            for c in contexts(&ch) {
                let m = (1.0 + st.chord * c.c + st.jack * c.j + st.stream * c.s)
                    * (1.0 + st.fatigue * (c.t_ms - t0) / 180_000.0);
                let s = st.sigma0 * st.spread * m;
                v += s * s * (1.0 + 8.0 * st.p_slip);
                let pm = st.miss_floor + st.miss_k * (c.c + c.j + c.s) / 3.0;
                let pr = p.judge.probs(st.bias, s, drift_var.sqrt(), st.p_slip);
                for i in 0..5 {
                    let pi = if i == 4 {
                        pm + (1.0 - pm) * pr[4]
                    } else {
                        (1.0 - pm) * pr[i]
                    };
                    exp[i] += pi;
                    var_exp[i] += pi * (1.0 - pi);
                }
            }
            let predicted = ((v / N as f64) + drift_var).sqrt() * 10.0;
            let ur = p.stats().ur;
            assert!(
                (ur / predicted - 1.0).abs() < 0.10,
                "{} seed {seed}: ur {ur:.1} vs predicted {predicted:.1}",
                st.name
            );
        }
        let mean = sum / cnt as f64;
        assert!(
            (mean - st.bias).abs() < 1.0,
            "{}: mean {mean:.2} bias {}",
            st.name,
            st.bias
        );
        for i in 0..5 {
            // 99% interval, widened a little: drift correlates neighbouring notes.
            let half = 3.5 * var_exp[i].sqrt();
            assert!(
                (got[i] - exp[i]).abs() <= half.max(3.0),
                "{} judgment {i}: got {} expected {:.1} +/- {half:.1}",
                st.name,
                got[i],
                exp[i]
            );
        }
    }
}

#[test]
fn same_seed_same_chart_same_errors() {
    let st = Style::builtin("Casual friend").unwrap();
    let (_, a, _) = run(&st, 9, 400, None);
    let (_, b, _) = run(&st, 9, 400, None);
    let (_, c, _) = run(&st, 10, 400, None);
    assert_eq!(a, b);
    assert_ne!(a, c);
}

#[test]
fn max_is_a_no_op() {
    let mut p = StylePlayer::new(Style::max(), &game(), 1, 100);
    assert!(p.is_max());
    for c in contexts(&chart(1, 100)) {
        assert_eq!(p.note(c), NoteDraw::default());
    }
}

#[test]
fn self_check_flags() {
    let st = Style::builtin("Casual friend").unwrap();
    let (p, _, _) = run(&st, 3, 600, Some(1.0));
    let s = p.stats();
    assert!(!s.uniform_like && !s.too_tight, "{s:?}");
    assert_eq!(s.hist.iter().sum::<u32>(), s.notes);
    // A tiny spread trips too-tight.
    let tiny = Style {
        sigma0: 0.5,
        p_slip: 0.0,
        focus_drift: 0.0,
        ..st
    };
    let (p, _, _) = run(&tiny, 3, 600, Some(1.0));
    assert!(p.stats().too_tight);
}

#[test]
fn model_errors_are_not_uniform_like() {
    let st = Style {
        sigma0: 1.0,
        ..Style::builtin("Sweaty").unwrap()
    };
    let mut p = StylePlayer::new(st, &game(), 1, 1000).with_fixed_k(1.0);
    for _ in 0..1000 {
        p.note(NoteCtx::default());
    }
    assert!(
        !p.stats().uniform_like,
        "normal errors are not uniform-like"
    );
}

#[test]
fn custom_name_and_files() {
    assert_eq!(custom_name("Sweaty"), "Custom (based on Sweaty)");
    assert_eq!(
        custom_name("Custom (based on Sweaty)"),
        "Custom (based on Sweaty)"
    );
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(list_styles(dir.path()).len(), 4);
    let mine = Style {
        name: custom_name("Rusty"),
        sigma0: 22.5,
        ..Style::builtin("Rusty").unwrap()
    };
    save_style(dir.path(), &mine).unwrap();
    let clash = Style {
        name: "sweaty".into(),
        ..mine.clone()
    };
    assert!(matches!(
        save_style(dir.path(), &clash),
        Err(StyleError::BuiltIn)
    ));
    let bad = Style {
        name: "../x".into(),
        ..mine.clone()
    };
    assert!(matches!(
        save_style(dir.path(), &bad),
        Err(StyleError::BadName)
    ));
    // A hand-made file named like a built-in is ignored.
    std::fs::write(dir.path().join("styles/Max.toml"), "sigma0 = 99").unwrap();
    let all = list_styles(dir.path());
    assert_eq!(all.len(), 5);
    assert_eq!(all[4], mine);
    assert_eq!(find_style(Some(dir.path()), &mine.name), mine);
    assert!(find_style(Some(dir.path()), "nope").is_max());
    delete_style(dir.path(), &mine.name).unwrap();
    assert_eq!(list_styles(dir.path()).len(), 4);
}
