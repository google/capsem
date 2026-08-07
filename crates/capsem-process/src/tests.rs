use super::*;
use clap::Parser;

// -----------------------------------------------------------------------
// Args parsing
// -----------------------------------------------------------------------

#[test]
fn args_parses_all_required() {
    let args = Args::try_parse_from([
        "capsem-process",
        "--id",
        "test-vm",
        "--assets-dir",
        "/tmp/assets",
        "--rootfs",
        "/tmp/rootfs.img",
        "--session-dir",
        "/tmp/session",
        "--active-profile",
        "/tmp/config/profiles/code",
        "--expected-kernel-hash",
        "aa",
        "--expected-initrd-hash",
        "bb",
        "--expected-rootfs-hash",
        "cc",
        "--uds-path",
        "/tmp/vm.sock",
    ])
    .unwrap();
    assert_eq!(args.id, "test-vm");
    assert_eq!(args.assets_dir, PathBuf::from("/tmp/assets"));
    assert_eq!(args.rootfs, PathBuf::from("/tmp/rootfs.img"));
    assert_eq!(args.session_dir, PathBuf::from("/tmp/session"));
    assert_eq!(
        args.active_profile,
        PathBuf::from("/tmp/config/profiles/code")
    );
    assert_eq!(args.uds_path, PathBuf::from("/tmp/vm.sock"));
}

#[test]
fn args_default_cpus() {
    let args = Args::try_parse_from([
        "capsem-process",
        "--id",
        "vm",
        "--assets-dir",
        "/a",
        "--rootfs",
        "/r",
        "--session-dir",
        "/s",
        "--active-profile",
        "/profiles/code",
        "--expected-kernel-hash",
        "aa",
        "--expected-initrd-hash",
        "bb",
        "--expected-rootfs-hash",
        "cc",
        "--uds-path",
        "/u",
    ])
    .unwrap();
    assert_eq!(args.cpus, 2);
}

#[test]
fn args_default_ram_mb() {
    let args = Args::try_parse_from([
        "capsem-process",
        "--id",
        "vm",
        "--assets-dir",
        "/a",
        "--rootfs",
        "/r",
        "--session-dir",
        "/s",
        "--active-profile",
        "/profiles/code",
        "--expected-kernel-hash",
        "aa",
        "--expected-initrd-hash",
        "bb",
        "--expected-rootfs-hash",
        "cc",
        "--uds-path",
        "/u",
    ])
    .unwrap();
    assert_eq!(args.ram_mb, 2048);
}

#[test]
fn args_default_scratch_disk_size_gb() {
    let args = Args::try_parse_from([
        "capsem-process",
        "--id",
        "vm",
        "--assets-dir",
        "/a",
        "--rootfs",
        "/r",
        "--session-dir",
        "/s",
        "--active-profile",
        "/profiles/code",
        "--expected-kernel-hash",
        "aa",
        "--expected-initrd-hash",
        "bb",
        "--expected-rootfs-hash",
        "cc",
        "--uds-path",
        "/u",
    ])
    .unwrap();
    assert_eq!(args.scratch_disk_size_gb, 16);
}

#[test]
fn args_custom_cpus_ram_and_scratch_disk_size() {
    let args = Args::try_parse_from([
        "capsem-process",
        "--id",
        "vm",
        "--assets-dir",
        "/a",
        "--rootfs",
        "/r",
        "--session-dir",
        "/s",
        "--active-profile",
        "/profiles/code",
        "--expected-kernel-hash",
        "aa",
        "--expected-initrd-hash",
        "bb",
        "--expected-rootfs-hash",
        "cc",
        "--uds-path",
        "/u",
        "--cpus",
        "8",
        "--ram-mb",
        "16384",
        "--scratch-disk-size-gb",
        "64",
    ])
    .unwrap();
    assert_eq!(args.cpus, 8);
    assert_eq!(args.ram_mb, 16384);
    assert_eq!(args.scratch_disk_size_gb, 64);
}

#[test]
fn prepare_session_layout_uses_requested_scratch_disk_size() {
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("session");

    let guest_dir = prepare_session_layout(&session_dir, 64).unwrap();

    assert_eq!(guest_dir, session_dir.join("guest"));
    let rootfs_img = guest_dir.join("system/rootfs.img");
    let metadata = std::fs::metadata(&rootfs_img).unwrap();
    assert_eq!(metadata.len(), 64 * 1024 * 1024 * 1024);
}

#[test]
fn args_missing_required_id_fails() {
    let result = Args::try_parse_from([
        "capsem-process",
        "--assets-dir",
        "/a",
        "--rootfs",
        "/r",
        "--session-dir",
        "/s",
        "--active-profile",
        "/profiles/code",
        "--expected-kernel-hash",
        "aa",
        "--expected-initrd-hash",
        "bb",
        "--expected-rootfs-hash",
        "cc",
        "--uds-path",
        "/u",
    ]);
    assert!(result.is_err());
}

#[test]
fn args_missing_required_assets_dir_fails() {
    let result = Args::try_parse_from([
        "capsem-process",
        "--id",
        "vm",
        "--rootfs",
        "/r",
        "--session-dir",
        "/s",
        "--active-profile",
        "/profiles/code",
        "--expected-kernel-hash",
        "aa",
        "--expected-initrd-hash",
        "bb",
        "--expected-rootfs-hash",
        "cc",
        "--uds-path",
        "/u",
    ]);
    assert!(result.is_err());
}

#[test]
fn args_missing_required_active_profile_fails() {
    let result = Args::try_parse_from([
        "capsem-process",
        "--id",
        "vm",
        "--assets-dir",
        "/a",
        "--rootfs",
        "/r",
        "--session-dir",
        "/s",
        "--expected-kernel-hash",
        "aa",
        "--expected-initrd-hash",
        "bb",
        "--expected-rootfs-hash",
        "cc",
        "--uds-path",
        "/u",
    ]);
    assert!(result.is_err());
}

#[test]
fn args_invalid_cpus_type_fails() {
    let result = Args::try_parse_from([
        "capsem-process",
        "--id",
        "vm",
        "--assets-dir",
        "/a",
        "--rootfs",
        "/r",
        "--session-dir",
        "/s",
        "--active-profile",
        "/profiles/code",
        "--expected-kernel-hash",
        "aa",
        "--expected-initrd-hash",
        "bb",
        "--expected-rootfs-hash",
        "cc",
        "--uds-path",
        "/u",
        "--cpus",
        "not-a-number",
    ]);
    assert!(result.is_err());
}

#[test]
fn args_checkpoint_path_optional() {
    let args = Args::try_parse_from([
        "capsem-process",
        "--id",
        "vm",
        "--assets-dir",
        "/a",
        "--rootfs",
        "/r",
        "--session-dir",
        "/s",
        "--active-profile",
        "/profiles/code",
        "--expected-kernel-hash",
        "aa",
        "--expected-initrd-hash",
        "bb",
        "--expected-rootfs-hash",
        "cc",
        "--uds-path",
        "/u",
    ])
    .unwrap();
    assert!(args.checkpoint_path.is_none());
}

#[test]
fn args_checkpoint_path_set() {
    let args = Args::try_parse_from([
        "capsem-process",
        "--id",
        "vm",
        "--assets-dir",
        "/a",
        "--rootfs",
        "/r",
        "--session-dir",
        "/s",
        "--active-profile",
        "/profiles/code",
        "--expected-kernel-hash",
        "aa",
        "--expected-initrd-hash",
        "bb",
        "--expected-rootfs-hash",
        "cc",
        "--uds-path",
        "/u",
        "--checkpoint-path",
        "/tmp/cp.vzsave",
    ])
    .unwrap();
    assert_eq!(
        args.checkpoint_path.unwrap(),
        PathBuf::from("/tmp/cp.vzsave")
    );
}

#[test]
fn args_env_vars_parsed() {
    let args = Args::try_parse_from([
        "capsem-process",
        "--id",
        "vm",
        "--assets-dir",
        "/a",
        "--rootfs",
        "/r",
        "--session-dir",
        "/s",
        "--active-profile",
        "/profiles/code",
        "--expected-kernel-hash",
        "aa",
        "--expected-initrd-hash",
        "bb",
        "--expected-rootfs-hash",
        "cc",
        "--uds-path",
        "/u",
        "--env",
        "FOO=bar",
        "--env",
        "BAZ=qux",
    ])
    .unwrap();
    assert_eq!(args.env, vec!["FOO=bar", "BAZ=qux"]);
}

// -----------------------------------------------------------------------
// CLI env parsing (used in run_async_main_loop)
// -----------------------------------------------------------------------

#[test]
fn cli_env_parsing_valid() {
    let env = ["FOO=bar".to_string(), "BAZ=qux=extra".to_string()];
    let parsed: Vec<(String, String)> = env
        .iter()
        .filter_map(|kv| {
            kv.split_once('=')
                .map(|(k, v)| (k.to_string(), v.to_string()))
        })
        .collect();
    assert_eq!(
        parsed,
        vec![
            ("FOO".to_string(), "bar".to_string()),
            ("BAZ".to_string(), "qux=extra".to_string()),
        ]
    );
}

#[test]
fn cli_env_parsing_no_equals_skipped() {
    let env = ["NOEQ".to_string(), "GOOD=val".to_string()];
    let parsed: Vec<(String, String)> = env
        .iter()
        .filter_map(|kv| {
            kv.split_once('=')
                .map(|(k, v)| (k.to_string(), v.to_string()))
        })
        .collect();
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0], ("GOOD".to_string(), "val".to_string()));
}

#[test]
fn cli_env_parsing_empty_value() {
    let env = ["KEY=".to_string()];
    let parsed: Vec<(String, String)> = env
        .iter()
        .filter_map(|kv| {
            kv.split_once('=')
                .map(|(k, v)| (k.to_string(), v.to_string()))
        })
        .collect();
    assert_eq!(parsed, vec![("KEY".to_string(), "".to_string())]);
}

// -----------------------------------------------------------------------
// trace_id generation: stitches together the three host-side processes
// (capsem-service, capsem-process, capsem-mcp-aggregator) so
// per-VM logs can be correlated across the process.log + the new
// mcp-aggregator.stderr.log streams.
// -----------------------------------------------------------------------

#[test]
fn generate_trace_id_is_16_hex_chars() {
    let id = generate_trace_id();
    assert_eq!(id.len(), 16);
    assert!(
        id.chars().all(|c| c.is_ascii_hexdigit()),
        "non-hex character in trace_id: {id}"
    );
}

#[test]
fn generate_trace_id_is_unique_across_calls() {
    // Not cryptographic -- but within a process, rapid successive
    // calls must not collide, otherwise correlation is useless.
    use std::collections::HashSet;
    let ids: HashSet<String> = (0..64).map(|_| generate_trace_id()).collect();
    assert_eq!(ids.len(), 64, "trace_id collisions: {ids:?}");
}

#[test]
fn aggregator_log_path_lives_in_session_dir() {
    let session = PathBuf::from("/tmp/some-session");
    let log = aggregator_log_path(&session);
    assert_eq!(log, session.join("mcp-aggregator.stderr.log"));
}

#[test]
fn process_kernel_cmdline_uses_arch_console_and_root_device() {
    let cmdline = process_kernel_cmdline();
    assert!(cmdline.contains("root=/dev/vda"));
    #[cfg(target_arch = "x86_64")]
    {
        assert!(cmdline.contains("console=ttyS0"));
        assert!(!cmdline.contains("console=hvc0"));
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        assert!(cmdline.contains("console=hvc0"));
    }
}

#[test]
fn missing_mcp_aggregator_fails_loud_instead_of_empty_stub() {
    let dir = tempfile::tempdir().unwrap();
    let fake_exe = dir.path().join("capsem-process");
    let error = resolve_mcp_aggregator_binary(&fake_exe)
        .expect_err("missing aggregator binary must not resolve");
    assert!(
        error.to_string().contains("capsem-mcp-aggregator"),
        "error should name the missing component: {error:#}"
    );
}

#[test]
fn mcp_aggregator_resolver_supports_cargo_test_deps_layout() {
    let dir = tempfile::tempdir().unwrap();
    let deps = dir.path().join("deps");
    std::fs::create_dir_all(&deps).unwrap();
    let aggregator = dir.path().join("capsem-mcp-aggregator");
    std::fs::write(&aggregator, "").unwrap();

    let resolved = resolve_mcp_aggregator_binary(&deps.join("capsem-process-test")).unwrap();
    assert_eq!(resolved, aggregator);
}
