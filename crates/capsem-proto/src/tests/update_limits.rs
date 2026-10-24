#[test]
fn update_execution_limits_have_one_typed_config_authority() {
    let config: toml::Value = toml::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/runtime-limits.toml"
    )))
    .unwrap();
    let limits = config
        .get("update")
        .expect("update execution limits must be config-owned");
    assert_eq!(limits["check_timeout_seconds"].as_integer(), Some(150));
    assert_eq!(limits["output_bytes_per_stream"].as_integer(), Some(65536));
    assert_eq!(
        limits["check_timeout_seconds"].as_integer().unwrap() as usize,
        crate::UPDATE_CHECK_TIMEOUT_SECONDS
    );
    assert_eq!(
        limits["output_bytes_per_stream"].as_integer().unwrap() as usize,
        crate::UPDATE_OUTPUT_BYTES_PER_STREAM
    );
}

#[path = "../../build/update_limits.rs"]
mod generated;

#[test]
fn invalid_update_limits_fail_before_a_binary_can_advertise_them() {
    for config in [
        "",
        "[update]\ncheck_timeout_seconds=0\noutput_bytes_per_stream=65536",
        "[update]\ncheck_timeout_seconds=150\noutput_bytes_per_stream=1048577",
        "[update]\ncheck_timeout_seconds=601\noutput_bytes_per_stream=65536",
        "[update]\ncheck_timeout_seconds=150\noutput_bytes_per_stream='65536'",
    ] {
        let value: toml::Value = toml::from_str(config).unwrap();
        assert!(std::panic::catch_unwind(|| generated::render(&value)).is_err());
    }
}
