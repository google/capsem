use std::path::Path;
use std::time::Duration;

use super::{parse, run};

const SAMPLE: &str = r#"{"metrics":{"cpu_s":{"unit":"seconds","samples":[0.14,0.15,0.16]}}}"#;

#[test]
fn a_sample_document_parses() {
    let collected = parse(SAMPLE).expect("parses");
    assert_eq!(collected.metrics.len(), 1);
    assert_eq!(collected.metrics["cpu_s"].samples, [0.14, 0.15, 0.16]);
    assert!(collected.sidecar.is_none());
}

#[test]
fn a_warning_printed_before_the_document_is_tolerated() {
    // A collector that prints a note to stdout before its JSON is common, and
    // refusing it would teach collectors to swallow their own diagnostics.
    let collected = parse(&format!("warming up...\n{SAMPLE}")).expect("parses");
    assert_eq!(collected.metrics.len(), 1);
}

#[test]
fn a_second_document_is_refused() {
    // Two documents mean the collector ran twice; only the first would ever be
    // read, and the second measurement would vanish silently.
    let error = parse(&format!("{SAMPLE}\n{SAMPLE}")).expect_err("must refuse");
    assert!(error.to_string().contains("more than one document"), "{error}");
}

#[test]
fn empty_output_is_refused() {
    assert!(parse("").is_err());
    assert!(parse("no json here\n").is_err());
}

#[test]
fn malformed_json_is_refused() {
    let error = parse("{\"metrics\": ").expect_err("must refuse");
    assert!(error.to_string().contains("not a valid sample document"), "{error}");
}

#[test]
fn an_unknown_field_is_refused() {
    let document = r#"{"metrics":{"a":{"unit":"seconds","samples":[1.0]}},"extra":1}"#;
    assert!(parse(document).is_err());
}

#[test]
fn a_collector_that_measured_nothing_is_refused() {
    // Rather than recording an empty document that would pass every ratchet.
    let error = parse(r#"{"metrics":{}}"#).expect_err("must refuse");
    assert!(error.to_string().contains("no metrics"), "{error}");
}

#[test]
fn a_metric_with_no_samples_is_refused() {
    let document = r#"{"metrics":{"cpu_s":{"unit":"seconds","samples":[]}}}"#;
    let error = parse(document).expect_err("must refuse");
    assert!(error.to_string().contains("no samples"), "{error}");
}

#[test]
fn a_sample_too_large_to_represent_is_refused() {
    // JSON cannot express NaN or infinity, and serde_json rejects a literal
    // that would overflow, so this never reaches the finite check in `parse`.
    // The check stays for callers that build a document in process; this
    // asserts the path a subprocess can actually take.
    let document = r#"{"metrics":{"cpu_s":{"unit":"seconds","samples":[1e400]}}}"#;
    let error = parse(document).expect_err("must refuse");
    assert!(
        format!("{error:#}").contains("not a valid sample document"),
        "{error:#}"
    );
}

#[test]
fn a_sidecar_is_carried_when_present() {
    let document = r#"{"metrics":{"a":{"unit":"count","samples":[1.0]}},"sidecar":"stdout.txt"}"#;
    assert_eq!(parse(document).expect("parses").sidecar.as_deref(), Some("stdout.txt"));
}

#[test]
fn a_collector_that_fails_is_reported_with_its_status() {
    let error = run(Path::new("false"), &[], Duration::from_secs(5)).expect_err("must refuse");
    assert!(error.to_string().contains("exited with"), "{error}");
}

#[test]
fn a_collector_that_prints_nothing_is_refused() {
    let error = run(Path::new("true"), &[], Duration::from_secs(5)).expect_err("must refuse");
    assert!(format!("{error:#}").contains("no JSON document"), "{error:#}");
}

#[test]
fn a_collector_that_hangs_is_killed() {
    // A collector that never exits would otherwise hold the machine lock the
    // whole gate runs under.
    let error = run(Path::new("sleep"), &["30".to_string()], Duration::from_millis(200)).expect_err("must time out");
    assert!(error.to_string().contains("did not finish"), "{error}");
}

#[test]
fn a_missing_collector_is_reported_by_name() {
    let error = run(Path::new("/nonexistent/collector"), &[], Duration::from_secs(5)).expect_err("must refuse");
    assert!(error.to_string().contains("/nonexistent/collector"), "{error}");
}

#[test]
fn a_real_collector_round_trips() {
    let error = run(Path::new("echo"), &[SAMPLE.to_string()], Duration::from_secs(5));
    let collected = error.expect("echo is a valid collector");
    assert_eq!(collected.metrics["cpu_s"].samples.len(), 3);
}
