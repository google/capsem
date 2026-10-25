use std::path::Path;
use std::time::Duration;

use crate::sample_limits::Limits;

fn parse(text: &str) -> anyhow::Result<super::Collected> {
    super::parse(text, Limits::configured())
}

fn run(program: &Path, args: &[String], timeout: Duration) -> anyhow::Result<super::Collected> {
    super::run(program, args, timeout, Limits::configured())
}

#[test]
fn metric_and_total_sample_limits_reject_instead_of_thinning() {
    let limits = Limits {
        max_collector_bytes: 1024,
        max_samples_per_metric: 2,
        max_samples_per_run: 3,
    };
    assert!(super::parse(SAMPLE, limits).is_err());
    let total = r#"{"metrics":{"a":{"unit":"count","samples":[1,2]},"b":{"unit":"count","samples":[3,4]}}}"#;
    assert!(super::parse(total, limits).is_err());
}

#[test]
fn exact_sample_and_byte_boundaries_preserve_every_observation() {
    let limits = Limits {
        max_collector_bytes: SAMPLE.len(),
        max_samples_per_metric: 3,
        max_samples_per_run: 3,
    };
    assert_eq!(
        super::parse(SAMPLE, limits).unwrap().metrics["cpu_s"].samples,
        [0.14, 0.15, 0.16]
    );
    assert!(super::parse(&format!(" {SAMPLE}"), limits).is_err());
    let duplicate = r#"{"metrics":{"a":{"unit":"count","samples":[1]},"a":{"unit":"count","samples":[2]}}}"#;
    assert!(parse(duplicate).is_err());
}

#[test]
fn oversized_live_output_is_refused_before_a_hanging_child_timeout() {
    let started = std::time::Instant::now();
    let limits = Limits {
        max_collector_bytes: 1024,
        max_samples_per_metric: 10,
        max_samples_per_run: 10,
    };
    let result = super::run(
        Path::new("sh"),
        &["-c".into(), "head -c 2048 /dev/zero; exec sleep 30".into()],
        Duration::from_secs(2),
        limits,
    );
    assert!(format!("{:#}", result.unwrap_err()).contains("byte limit"));
    assert!(started.elapsed() < Duration::from_secs(1));
}

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

const FLOOD_THEN_DOCUMENT: &str = "head -c 204800 /dev/zero | tr '\\0' a; echo; printf '%s\\n' \"$1\"";

#[test]
fn a_collector_that_prints_more_than_a_pipe_buffer_completes_promptly() {
    // stdout is a pipe with a 64 KiB buffer. A collector that prints more
    // than that before exiting blocks on write until someone reads, and a
    // runner that only reads after exit turns every chatty collector into a
    // reported timeout.
    let started = std::time::Instant::now();
    let collected = run(
        Path::new("sh"),
        &[
            "-c".to_string(),
            FLOOD_THEN_DOCUMENT.to_string(),
            "sh".to_string(),
            SAMPLE.to_string(),
        ],
        Duration::from_secs(20),
    )
    .expect("a chatty collector still completes");
    assert_eq!(collected.metrics["cpu_s"].samples.len(), 3);
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "must not wait for the timeout: {:?}",
        started.elapsed()
    );
}

#[test]
fn a_collector_that_floods_stdout_then_hangs_is_still_killed() {
    let script = "head -c 204800 /dev/zero | tr '\\0' a; exec sleep 30";
    let started = std::time::Instant::now();
    let error = run(
        Path::new("sh"),
        &["-c".to_string(), script.to_string()],
        Duration::from_millis(500),
    )
    .expect_err("must time out");
    assert!(error.to_string().contains("did not finish"), "{error}");
    assert!(started.elapsed() < Duration::from_secs(10), "{:?}", started.elapsed());
}

#[test]
fn a_collector_that_floods_stdout_and_fails_reports_its_status() {
    let script = "head -c 204800 /dev/zero | tr '\\0' a; exit 3";
    let error = run(
        Path::new("sh"),
        &["-c".to_string(), script.to_string()],
        Duration::from_secs(20),
    )
    .expect_err("must refuse");
    assert!(error.to_string().contains("exited with 3"), "{error}");
}

#[test]
fn a_grandchild_holding_stdout_open_after_exit_is_bounded_by_the_deadline() {
    // The collector exits cleanly, but a background process it started still
    // holds the stdout pipe; EOF would arrive only when that process ends.
    let script = "sleep 5 & printf '%s\\n' \"$1\"; exit 0";
    let started = std::time::Instant::now();
    let error = run(
        Path::new("sh"),
        &[
            "-c".to_string(),
            script.to_string(),
            "sh".to_string(),
            SAMPLE.to_string(),
        ],
        Duration::from_millis(500),
    )
    .expect_err("must not wait for the grandchild");
    assert!(error.to_string().contains("stdout stayed open"), "{error}");
    assert!(started.elapsed() < Duration::from_secs(10), "{:?}", started.elapsed());
}

#[test]
fn collector_completion_cancels_background_work_on_every_exit_path() {
    for (ending, redirect, succeeds) in [
        ("wait", "", false),
        ("exit 0", "", false),
        ("exit 0", ">/dev/null 2>&1", true),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join("unowned-work");
        let trigger = directory.path().join("collector-returned");
        let script = format!(
            "(while [ ! -e \"$3\" ]; do sleep 0.01; done; printf leaked > \"$1\") {redirect} & printf '%s\\n' \"$2\"; {ending}"
        );
        let result = run(
            Path::new("sh"),
            &[
                "-c".into(),
                script,
                "sh".into(),
                marker.to_str().unwrap().into(),
                SAMPLE.into(),
                trigger.to_str().unwrap().into(),
            ],
            if succeeds {
                Duration::from_secs(5)
            } else {
                Duration::from_millis(100)
            },
        );
        assert_eq!(result.is_ok(), succeeds, "{ending}: {result:?}");
        std::fs::write(trigger, "returned").unwrap();
        std::thread::sleep(Duration::from_millis(700));
        assert!(!marker.exists(), "collector left background work after {ending}");
    }
}
