use super::{assess, Objection};

fn reasons(objections: &[Objection]) -> Vec<&str> {
    objections.iter().map(|o| o.what.as_str()).collect()
}

#[test]
fn an_idle_machine_with_a_fixed_clock_is_fit() {
    assert!(assess(16, Some(0.2), Some("performance"), true, &[]).is_empty());
}

#[test]
fn load_is_judged_per_core_not_absolutely() {
    // Load 4 is idle on a 32-way box and saturated on a dual. Judging it
    // absolutely would either refuse every big machine or accept every busy
    // small one.
    assert!(assess(32, Some(4.0), Some("performance"), true, &[]).is_empty());
    let small = assess(2, Some(4.0), Some("performance"), true, &[]);
    assert_eq!(reasons(&small), ["load"]);
}

#[test]
fn a_varying_clock_is_refused() {
    for name in ["powersave", "ondemand", "conservative", "schedutil"] {
        let objections = assess(16, Some(0.1), Some(name), true, &[]);
        assert_eq!(reasons(&objections), ["governor"], "{name} was accepted");
    }
}

#[test]
fn a_fixed_clock_is_accepted() {
    assert!(assess(16, Some(0.1), Some("performance"), true, &[]).is_empty());
}

#[test]
fn an_absent_governor_is_not_an_objection() {
    // macOS publishes none. Absence of the fact is not evidence of a problem.
    assert!(assess(16, Some(0.1), None, true, &[]).is_empty());
}

#[test]
fn an_absent_load_average_is_not_an_objection() {
    assert!(assess(16, None, Some("performance"), true, &[]).is_empty());
}

#[test]
fn a_machine_without_kvm_is_refused() {
    let objections = assess(16, Some(0.1), Some("performance"), false, &[]);
    assert_eq!(reasons(&objections), ["kvm"]);
}

#[test]
fn processes_already_running_are_refused_and_named() {
    let strays = vec!["capsem-service".to_string(), "capsem-gateway".to_string()];
    let objections = assess(16, Some(0.1), Some("performance"), true, &strays);
    assert_eq!(reasons(&objections), ["strays"]);
    // Naming them is the difference between a refusal someone can act on and
    // one they will work around.
    assert!(objections[0].detail.contains("capsem-service"));
    assert!(objections[0].detail.contains("capsem-gateway"));
}

#[test]
fn every_objection_is_reported_not_just_the_first() {
    let strays = vec!["capsem-service".to_string()];
    let objections = assess(2, Some(9.0), Some("powersave"), false, &strays);
    assert_eq!(reasons(&objections), ["load", "governor", "kvm", "strays"]);
}

#[test]
fn an_objection_explains_itself_in_the_message() {
    let objections = assess(2, Some(9.0), Some("performance"), true, &[]);
    let detail = &objections[0].detail;
    assert!(detail.contains("9.00"), "{detail}");
    assert!(detail.contains("2 cores"), "{detail}");
}

#[test]
fn the_doctor_does_not_report_itself_as_contention() {
    // `capsem-bench-rs` matches `^capsem`, so without this it would find
    // itself and declare every machine unfit.
    let listing = "111 capsem-service\n222 capsem-bench-rs\n333 capsem-gateway\n";
    let strays = super::strays_from_pgrep(listing, "222");
    assert_eq!(strays, ["capsem-service", "capsem-gateway"]);
}

#[test]
fn an_empty_listing_finds_nothing() {
    assert!(super::strays_from_pgrep("", "222").is_empty());
}

#[test]
fn a_malformed_line_is_skipped_rather_than_panicking() {
    let strays = super::strays_from_pgrep("garbage\n111 capsem-service\n", "999");
    assert_eq!(strays, ["capsem-service"]);
}
