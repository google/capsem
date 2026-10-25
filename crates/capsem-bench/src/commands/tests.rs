use super::*;

const PROFILE: &str = include_str!("../../../../config/gate.toml");

const SHA: &str = "0123456789012345678901234567890123456789";

fn fitness(fit: bool) -> machine::Fitness {
    machine::Fitness {
        host: schema::Host {
            arch: "x86_64".into(),
            os: "linux".into(),
            cpu_count: 8,
            kvm: true,
            governor: None,
            load_before: 0.0,
        },
        objections: if fit {
            vec![]
        } else {
            vec![machine::Objection {
                what: "load".into(),
                detail: "busy".into(),
            }]
        },
    }
}

fn workload(root: &Path, name: &str) {
    std::fs::write(
        root.join(name),
        format!(
            "touch '{}'; echo '{{\"metrics\":{{\"value\":{{\"unit\":\"count\",\"samples\":[1]}}}}}}'",
            root.join("ran").display()
        ),
    )
    .unwrap();
}

#[test]
fn unfit_admission_never_launches_the_workload() {
    let root = tempfile::tempdir().unwrap();
    workload(root.path(), "routes");
    let result = run_with_probe(
        &[schema::Dimension::Routes],
        root.path(),
        &root.path().join("store.db"),
        Duration::from_secs(2),
        Some("/bin/sh"),
        false,
        "test",
        SHA,
        PROFILE,
        || Ok(fitness(false)),
    );
    assert!(result.unwrap_err().to_string().contains("unfit before"));
    assert!(!root.path().join("ran").exists());
}

#[test]
fn contaminated_observations_are_retained_but_never_authority() {
    let root = tempfile::tempdir().unwrap();
    workload(root.path(), "routes");
    let path = root.path().join("store.db");
    let mut checks = [fitness(true), fitness(false)].into_iter();
    let result = run_with_probe(
        &[schema::Dimension::Routes],
        root.path(),
        &path,
        Duration::from_secs(2),
        Some("/bin/sh"),
        false,
        "test",
        SHA,
        PROFILE,
        || Ok(checks.next().unwrap()),
    );
    assert!(result.unwrap_err().to_string().contains("diagnostic"));
    let db = store::open(&path).unwrap();
    let record = store::latest(&db, schema::Dimension::Routes, "x86_64")
        .unwrap()
        .unwrap();
    assert_eq!(record.metrics[0].summary.n, 1);
    assert!(crate::evidence::eligible(&record).is_err());
}

#[test]
fn each_dimension_receives_fresh_checks_after_child_cleanup() {
    let root = tempfile::tempdir().unwrap();
    workload(root.path(), "routes");
    workload(root.path(), "stream");
    let mut calls = 0;
    run_with_probe(
        &[schema::Dimension::Routes, schema::Dimension::Stream],
        root.path(),
        &root.path().join("store.db"),
        Duration::from_secs(2),
        Some("/bin/sh"),
        false,
        "test",
        SHA,
        PROFILE,
        || {
            calls += 1;
            Ok(fitness(true))
        },
    )
    .unwrap();
    assert_eq!(calls, 4);
}

#[test]
fn quick_run_is_diagnostic_even_on_a_busy_machine() {
    let root = tempfile::tempdir().unwrap();
    workload(root.path(), "routes");
    run_with_probe(
        &[schema::Dimension::Routes, schema::Dimension::Stream],
        root.path(),
        &root.path().join("store.db"),
        Duration::from_secs(2),
        Some("/bin/sh"),
        true,
        "test",
        "unknown",
        PROFILE,
        || Ok(fitness(false)),
    )
    .unwrap();
    let db = store::open(&root.path().join("store.db")).unwrap();
    assert!(store::subjects(&db).unwrap().is_empty());
}

#[test]
fn missing_requested_collector_fails_before_any_child_or_store() {
    let root = tempfile::tempdir().unwrap();
    let marker = root.path().join("ran");
    std::fs::write(
        root.path().join("routes"),
        format!(
            "touch '{}'; echo '{{\"metrics\":{{\"value\":{{\"unit\":\"count\",\"samples\":[1]}}}}}}'",
            marker.display()
        ),
    )
    .unwrap();
    let store = root.path().join("measurements.db");
    let result = run_dimensions(
        &[schema::Dimension::Routes, schema::Dimension::Stream],
        root.path(),
        &store,
        Duration::from_secs(2),
        Some("/bin/sh"),
        false,
        "test",
        "head",
        "profile",
    );
    let error = result.unwrap_err().to_string();
    assert!(error.contains("stream"), "{error}");
    assert!(!marker.exists(), "partial workloads must not run before admission");
    assert!(!store.exists(), "failed admission must not produce a partial record");
}

#[test]
fn default_dimension_inventory_names_missing_collectors() {
    let root = tempfile::tempdir().unwrap();
    let error = run_dimensions(
        schema::Dimension::ALL,
        root.path(),
        &root.path().join("store.db"),
        Duration::from_secs(2),
        None,
        false,
        "test",
        "head",
        "profile",
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("routes") && error.contains("vsock"), "{error}");
}

#[test]
fn install_collector_receives_the_exact_source_recorded_with_fresh_fitness() {
    let root = tempfile::tempdir().unwrap();
    let phase = |source: &str, digest: &str| {
        serde_json::json!({"source_commit":source,"version":if source==SHA {"0.7.1"} else {"0.7.0"},
            "package_sha256":digest.repeat(64),"sbom_sha256":digest.repeat(64),
            "manifest_sha256":digest.repeat(64),"runtime_sha256":"c".repeat(64)})
    };
    let sidecar = serde_json::json!({"schema":"capsem.bench.install.v1","scope":"binary_apply_only",
        "source_commit":SHA,"owner":"linux-docker-systemd","transport":"service_uds","before":phase(&"b".repeat(40),"b"),
        "after":phase(SHA,"a"),"receipt_sha256":"d".repeat(64)})
    .to_string();
    let document =
        serde_json::json!({"metrics":{"activation.seconds":{"unit":"seconds","samples":[1]}},"sidecar":sidecar});
    std::fs::write(
        root.path().join("install"),
        format!(
            "test \"$1\" = --source-commit && test \"$2\" = {SHA} || exit 23; \
             echo '{document}'"
        ),
    )
    .unwrap();
    let path = root.path().join("store.db");
    let mut probes = 0;
    run_with_probe(
        &[schema::Dimension::Install],
        root.path(),
        &path,
        Duration::from_secs(2),
        Some("/bin/sh"),
        false,
        "test",
        SHA,
        PROFILE,
        || {
            probes += 1;
            Ok(fitness(true))
        },
    )
    .unwrap();
    assert_eq!(probes, 2);
    let record = store::latest(&store::open(&path).unwrap(), schema::Dimension::Install, "x86_64")
        .unwrap()
        .unwrap();
    assert_eq!(record.release.commit, SHA);
    assert_eq!(record.metrics[0].samples.as_deref(), Some(&[1.0][..]));
}
