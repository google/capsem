use super::*;

#[test]
fn collector_preserves_sub_millisecond_samples_in_the_declared_unit() {
    let wire = collector_metrics(BTreeMap::from([("oneshot.wall_ns".into(), vec![490_000.0])]));
    assert_eq!(wire["oneshot.wall_ns"]["unit"], "nanoseconds");
    assert_eq!(wire["oneshot.wall_ns"]["samples"][0], 490_000.0);
}

#[test]
fn one_shot_candidates_are_input_sensitive_across_simd_boundaries() {
    for size in [1, 16, 32, 64, 128, 240, 256, 1024, 65_536] {
        let mut input = vec![0xa5; size];
        for algorithm in ["blake3-256", "xxh3-128"] {
            let before = fingerprint(algorithm, &input);
            input[size - 1] ^= 0xff;
            assert_ne!(before, fingerprint(algorithm, &input), "{algorithm}, {size}");
        }
    }
}
