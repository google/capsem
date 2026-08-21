use super::{compare, Statistic, Summary};

/// 1..=100 has percentiles that can be checked by hand.
fn hundred() -> Vec<f64> {
    (1..=100).map(f64::from).collect()
}

#[test]
fn an_empty_sample_set_has_no_summary() {
    // Not a zero: a collector that emitted nothing would otherwise pass every
    // ratchet forever.
    assert!(Summary::of(&[]).is_none());
}

#[test]
fn one_sample_is_its_own_every_statistic() {
    let summary = Summary::of(&[4.2]).expect("one sample summarizes");
    assert_eq!(summary.n, 1);
    for statistic in [
        Statistic::Min,
        Statistic::Max,
        Statistic::Mean,
        Statistic::Median,
        Statistic::P90,
        Statistic::P99,
        Statistic::P999,
    ] {
        assert_eq!(summary.at(statistic), 4.2);
    }
    assert_eq!(summary.stddev, 0.0);
    assert_eq!(summary.cv, 0.0);
}

#[test]
fn identical_samples_have_no_spread() {
    let summary = Summary::of(&[7.0; 50]).expect("summarizes");
    assert_eq!(summary.stddev, 0.0);
    assert_eq!(summary.cv, 0.0);
    assert_eq!(summary.mad, 0.0);
    assert_eq!(summary.median, 7.0);
}

#[test]
fn percentiles_interpolate_rather_than_jump() {
    let summary = Summary::of(&hundred()).expect("summarizes");
    assert_eq!(summary.min, 1.0);
    assert_eq!(summary.max, 100.0);
    assert!((summary.mean - 50.5).abs() < 1e-9);
    assert!((summary.median - 50.5).abs() < 1e-9);
    assert!((summary.p90 - 90.1).abs() < 1e-9);
    assert!((summary.p95 - 95.05).abs() < 1e-9);
    assert!((summary.p99 - 99.01).abs() < 1e-9);
}

#[test]
fn p999_below_a_thousand_samples_is_not_just_the_maximum() {
    // Nearest-rank would return 100.0 here and hide the tail it exists to
    // describe. 384 samples is the hot-route window, so this is the real case.
    let summary = Summary::of(&hundred()).expect("summarizes");
    assert!(summary.p999 < summary.max, "p999 collapsed onto the maximum");
    assert!(summary.p999 > summary.p99);
}

#[test]
fn samples_need_not_arrive_sorted() {
    let mut shuffled = hundred();
    shuffled.reverse();
    assert_eq!(
        Summary::of(&shuffled).expect("summarizes"),
        Summary::of(&hundred()).expect("summarizes")
    );
}

#[test]
fn mad_ignores_the_outlier_that_drags_stddev() {
    let mut samples = vec![10.0; 99];
    samples.push(1000.0);
    let summary = Summary::of(&samples).expect("summarizes");
    assert_eq!(summary.mad, 0.0, "the median deviation is unmoved");
    assert!(summary.stddev > 90.0, "stddev absorbs the outlier");
}

#[test]
fn a_zero_mean_does_not_make_the_spread_infinite() {
    let summary = Summary::of(&[0.0, 0.0, 0.0]).expect("summarizes");
    assert_eq!(summary.cv, 0.0);
    assert!(summary.cv.is_finite());
}

fn steady(value: f64) -> Summary {
    Summary::of(&[value; 64]).expect("summarizes")
}

#[test]
fn growth_inside_the_allowed_ratio_is_not_a_regression() {
    let verdict = compare(
        "routes.gateway./vms/list.cpu_s",
        Statistic::Median,
        &steady(0.10),
        &steady(0.11),
        1.2,
        1.0,
    );
    assert!(!verdict.regressed);
    assert!(!verdict.significant);
    assert!((verdict.ratio - 1.1).abs() < 1e-9);
    assert!((verdict.delta_pct - 10.0).abs() < 1e-9);
}

#[test]
fn growth_past_the_ratio_on_a_quiet_metric_is_significant() {
    let verdict = compare(
        "routes.gateway./vms/list.cpu_s",
        Statistic::Median,
        &steady(0.10),
        &steady(0.20),
        1.2,
        1.0,
    );
    assert!(verdict.regressed);
    assert!(verdict.significant, "a doubling on a flat baseline is real");
}

#[test]
fn the_same_move_on_a_noisy_baseline_is_not_significant() {
    // The 0.6.0 false alarm in miniature: the ratio is breached, but the
    // evidence itself wobbles at least as much, so the run has not learned
    // anything and must not hold a release.
    let noisy = Summary::of(&[0.02, 0.18, 0.04, 0.16, 0.10, 0.06, 0.14, 0.10])
        .expect("summarizes");
    let verdict = compare(
        "routes.gateway./vms/list.cpu_s",
        Statistic::Median,
        &noisy,
        &steady(0.13),
        1.2,
        1.0,
    );
    assert!(verdict.regressed, "the ratio is still breached");
    assert!(
        !verdict.significant,
        "a move inside the baseline's own spread is not a finding"
    );
}

#[test]
fn a_zero_baseline_reports_infinity_rather_than_no_change() {
    let verdict = compare(
        "criterion.dns_cache.hit_ns",
        Statistic::Mean,
        &steady(0.0),
        &steady(5.0),
        1.2,
        1.0,
    );
    assert!(verdict.ratio.is_infinite());
    assert!(verdict.regressed);
}

#[test]
fn two_zeroes_are_unchanged() {
    let verdict = compare(
        "routes.service./version.errors",
        Statistic::Mean,
        &steady(0.0),
        &steady(0.0),
        1.2,
        1.0,
    );
    assert_eq!(verdict.ratio, 1.0);
    assert!(!verdict.regressed);
}

#[test]
fn an_improvement_is_never_a_regression() {
    let verdict = compare(
        "lifecycle.provision_ms",
        Statistic::Median,
        &steady(900.0),
        &steady(450.0),
        1.2,
        1.0,
    );
    assert!(!verdict.regressed);
    assert!(verdict.delta_abs < 0.0);
    assert!((verdict.delta_pct + 50.0).abs() < 1e-9);
}

#[test]
fn statistics_are_rounded_so_a_record_stores_stably() {
    // Full f64 precision is not just noise, it is unstable: serde_json writes
    // it exactly and parses it back up to one ULP off, so evidence bytes and
    // therefore evidence digests changed on every read.
    let summary = Summary::of(&[0.14, 0.15, 0.16]).expect("summarizes");
    for value in [summary.mean, summary.stddev, summary.cv, summary.mad] {
        let text = serde_json::to_string(&value).expect("serializes");
        let parsed: f64 = serde_json::from_str(&text).expect("parses");
        assert_eq!(parsed, value, "{text} did not survive a round trip");
        assert!(text.len() <= 12, "{text} carries more digits than measured");
    }
}

#[test]
fn statistics_round_to_hundredths() {
    assert_eq!(Summary::of(&[1234.5678_f64]).expect("s").mean, 1234.57);
    assert_eq!(Summary::of(&[0.156_f64]).expect("s").mean, 0.16);
    assert_eq!(Summary::of(&[987654321.0_f64]).expect("s").mean, 987654321.0);
}

#[test]
fn rounding_leaves_zero_and_negatives_alone() {
    let zero = Summary::of(&[0.0]).expect("summarizes");
    assert_eq!(zero.mean, 0.0);
    let negative = Summary::of(&[-1.5, -2.5]).expect("summarizes");
    assert_eq!(negative.mean, -2.0);
    assert_eq!(negative.min, -2.5);
}
