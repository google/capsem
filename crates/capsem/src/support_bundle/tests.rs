//! Tests for the support bundler. Use a fake `~/.capsem/` layout in a
//! tempdir, point CAPSEM_HOME at it, run `support_bundle::run`, and
//! inspect the emitted tar.gz.

use std::fs;
use std::io::Read;
use std::path::Path;
use tempfile::TempDir;

fn write(p: &Path, content: &[u8]) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, content).unwrap();
}

fn read_tar_entries(path: &Path) -> Vec<(String, Vec<u8>)> {
    let f = fs::File::open(path).unwrap();
    let gz = flate2::read::GzDecoder::new(f);
    let mut tar = tar::Archive::new(gz);
    let mut out = Vec::new();
    for e in tar.entries().unwrap() {
        let mut e = e.unwrap();
        let path = e.path().unwrap().to_string_lossy().to_string();
        let mut buf = Vec::new();
        e.read_to_end(&mut buf).unwrap();
        out.push((path, buf));
    }
    out
}

fn fake_capsem_home() -> TempDir {
    let dir = TempDir::new().unwrap();
    // One call sets every Capsem path variable from this root. Setting only
    // CAPSEM_HOME left production code reading the caller's run directory.
    // Held for the rest of the process on purpose: every test serializes on
    // lock_test_env and re-redirects, so restoring between fixtures would only
    // reintroduce the ambient values this exists to shut out.
    std::mem::forget(capsem_foundation::paths::CapsemPathsGuard::redirect(dir.path()));
    let home = dir.path();
    fs::create_dir_all(home.join("run")).unwrap();
    fs::create_dir_all(home.join("logs")).unwrap();
    fs::create_dir_all(home.join("sessions")).unwrap();
    write(
        &home.join("run/service.log"),
        b"INFO service line one\nINFO service line two\n",
    );
    write(&home.join("run/mcp.log"), b"INFO mcp starting\n");
    // Raw stderr lives in its own directory on every install; the rotation
    // pruner owns run/ and would otherwise be free to delete it.
    write(&home.join("run/stderr/service.log"), b"");
    write(&home.join("run/gateway.pid"), b"12345");
    write(&home.join("run/gateway.port"), b"19222");
    write(
        &home.join("settings.toml"),
        br#"[provider.anthropic]
api_key = "sk-ant-real-secret-here-very-long-string"
endpoint = "https://api.anthropic.com"
"#,
    );
    write(&home.join("logs/20260502-180000.jsonl"), b"{\"level\":\"info\"}\n");
    dir
}

#[test]
fn bundle_happy_path_writes_tar_gz_with_manifest() {
    let _g = crate::lock_test_env();
    let _dir = fake_capsem_home();
    let out = crate::support_bundle::run(None, 0, false, false).unwrap();
    assert!(out.exists(), "{}", out.display());
    let entries = read_tar_entries(&out);

    let manifest_entry = entries.iter().find(|(p, _)| p.ends_with("manifest.json"));
    assert!(manifest_entry.is_some(), "manifest.json missing");
    let manifest_text = std::str::from_utf8(&manifest_entry.unwrap().1).unwrap();
    let manifest: serde_json::Value = serde_json::from_str(manifest_text).unwrap();
    assert_eq!(manifest["schema_version"], 1);
    assert_eq!(manifest["redacted"], true);
}

#[test]
fn bundle_redacts_secrets_in_settings_toml() {
    let _g = crate::lock_test_env();
    let _dir = fake_capsem_home();
    let out = crate::support_bundle::run(None, 0, false, false).unwrap();
    let entries = read_tar_entries(&out);

    let settings_toml_entry = entries
        .iter()
        .find(|(p, _)| p.ends_with("config/settings.toml"))
        .expect("config/settings.toml should be in bundle");
    let text = std::str::from_utf8(&settings_toml_entry.1).unwrap();
    assert!(
        !text.contains("sk-ant-real-secret-here-very-long-string"),
        "secret leaked: {text}"
    );
    assert!(text.contains("<redacted>"), "redaction marker missing: {text}");
    // Non-secret value preserved:
    assert!(text.contains("https://api.anthropic.com"));
}

#[test]
fn bundle_no_redact_keeps_secrets() {
    let _g = crate::lock_test_env();
    let _dir = fake_capsem_home();
    let out = crate::support_bundle::run(None, 0, false, true /*no_redact*/).unwrap();
    let entries = read_tar_entries(&out);

    let settings_toml_entry = entries
        .iter()
        .find(|(p, _)| p.ends_with("config/settings.toml"))
        .unwrap();
    let text = std::str::from_utf8(&settings_toml_entry.1).unwrap();
    assert!(
        text.contains("sk-ant-real-secret-here-very-long-string"),
        "no-redact should preserve: {text}"
    );
}

#[test]
fn bundle_excludes_gateway_token_even_when_present() {
    let _g = crate::lock_test_env();
    let dir = fake_capsem_home();
    let home = dir.path();
    // Plant a gateway.token to make sure it's NOT in the bundle.
    write(&home.join("run/gateway.token"), b"tok-this-must-not-leak-abcd1234");

    let out = crate::support_bundle::run(None, 0, false, false).unwrap();
    let entries = read_tar_entries(&out);

    for (p, b) in &entries {
        let text = std::str::from_utf8(b).unwrap_or("");
        assert!(
            !text.contains("tok-this-must-not-leak-abcd1234"),
            "gateway.token leaked into {p}"
        );
    }
}

#[test]
fn bundle_marks_missing_files_in_manifest() {
    let _g = crate::lock_test_env();
    let _dir = fake_capsem_home();
    // CAPSEM_HOME has no gateway.log, no tray.log -- expect missing entries.
    let out = crate::support_bundle::run(None, 0, false, false).unwrap();
    let entries = read_tar_entries(&out);

    let manifest_text =
        std::str::from_utf8(&entries.iter().find(|(p, _)| p.ends_with("manifest.json")).unwrap().1).unwrap();
    let manifest: serde_json::Value = serde_json::from_str(manifest_text).unwrap();
    let sections = manifest["sections"].as_array().unwrap();
    let gateway_section = sections
        .iter()
        .find(|s| s["path"].as_str().unwrap_or("").ends_with("gateway.log"))
        .expect("gateway.log section missing");
    assert_eq!(gateway_section["missing"], true);
}

#[test]
fn bundle_includes_asset_manifest_metadata() {
    let _g = crate::lock_test_env();
    let dir = fake_capsem_home();
    let home = dir.path();
    write(
        &home.join("assets/manifest.json"),
        br#"{"format":2,"refresh_policy":"24h","assets":{"current":"2026.0613.1","releases":{}},"binaries":{"current":"1.3.0","releases":{}}}"#,
    );
    write(
        &home.join("assets/manifest-metadata.json"),
        br#"{"schema":"capsem.manifest_metadata.v1","origin":"package","manifest_url":"file:///tmp/corp/manifest.json","packaged_at":"2026-06-13T00:00:00Z"}"#,
    );

    let out = crate::support_bundle::run(None, 0, false, false).unwrap();
    let entries = read_tar_entries(&out);

    let origin_entry = entries
        .iter()
        .find(|(p, _)| p.ends_with("assets/manifest-metadata.json"))
        .expect("asset manifest metadata provenance should be in support bundle");
    let origin: serde_json::Value = serde_json::from_slice(&origin_entry.1).unwrap();
    assert_eq!(origin["schema"], "capsem.manifest_metadata.v1");
    assert_eq!(origin["origin"], "package");
    assert_eq!(origin["manifest_url"], "file:///tmp/corp/manifest.json");

    let manifest_text = std::str::from_utf8(
        &entries
            .iter()
            .find(|(p, _)| p.ends_with("/manifest.json") && !p.contains("/assets/"))
            .unwrap()
            .1,
    )
    .unwrap();
    let manifest: serde_json::Value = serde_json::from_str(manifest_text).unwrap();
    let sections = manifest["sections"].as_array().unwrap();
    assert!(
        sections.iter().any(|section| {
            section["path"]
                .as_str()
                .is_some_and(|path| path.ends_with("assets/manifest-metadata.json"))
                && section["missing"].as_bool() != Some(true)
                && section["kind"].as_str() == Some("json")
        }),
        "manifest-metadata section missing from support manifest: {sections:#?}"
    );
}

#[test]
fn bundle_includes_runtime_boundary_debug_contract() {
    let _g = crate::lock_test_env();
    let _dir = fake_capsem_home();
    let out = crate::support_bundle::run(None, 0, false, false).unwrap();
    let entries = read_tar_entries(&out);

    let boundary_entry = entries
        .iter()
        .find(|(p, _)| p.ends_with("system/runtime-boundary.json"))
        .expect("runtime boundary debug contract should be in bundle");
    let boundary: serde_json::Value = serde_json::from_slice(&boundary_entry.1).unwrap();
    let services = boundary["host_vsock_services"].as_array().unwrap();
    assert!(
        services.iter().any(|s| s["service"] == "audit" && s["port"] == 5006),
        "audit VSOCK service must be first-party in debug output: {boundary}"
    );
    assert!(
        boundary["closed_raw_vsock_ports"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["port"] == 5003 && p["reason"] == "retired_mcp_raw_port"),
        "retired raw MCP port must be called out as closed: {boundary}"
    );
    assert!(
        boundary["debug_routes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|route| route == "/triage"),
        "debug route inventory should include /triage: {boundary}"
    );
    let routes = boundary["debug_routes"].as_array().unwrap();
    for route in [
        "/assets/status",
        "/plugins/list",
        "/plugins/{plugin_id}/info",
        "/plugins/credential_broker/credentials/info",
        "/mcp/info",
        "/mcp/default/info",
        "/mcp/servers/list",
    ] {
        assert!(
            routes.iter().any(|candidate| candidate == route),
            "runtime boundary debug contract missing {route}: {boundary}"
        );
    }
    assert!(
        !routes
            .iter()
            .any(|route| route.as_str().is_some_and(|route| route.starts_with("/profiles"))),
        "profiles are gone; the debug contract must not advertise a /profiles route: {boundary}"
    );
}

#[test]
fn bundle_includes_supply_chain_debug_references() {
    let _g = crate::lock_test_env();
    let _dir = fake_capsem_home();
    let out = crate::support_bundle::run(None, 0, false, false).unwrap();
    let entries = read_tar_entries(&out);

    let supply_chain_entry = entries
        .iter()
        .find(|(p, _)| p.ends_with("system/supply-chain.json"))
        .expect("support bundle should include supply-chain debug references");
    let supply_chain: serde_json::Value = serde_json::from_slice(&supply_chain_entry.1).unwrap();
    assert_eq!(supply_chain["host_sbom"]["format"], "spdx_json_2_3");
    assert_eq!(supply_chain["host_sbom"]["release_artifact"], "capsem-sbom.spdx.json");
    assert_eq!(supply_chain["host_sbom"]["scope"], "host_binaries");
    assert_eq!(supply_chain["host_sbom"]["attestation"], "github_attestations");
    assert_eq!(supply_chain["vm_obom"]["scope"], "base_image");
    assert!(supply_chain.get("profile_obom").is_none());
    assert_eq!(supply_chain["manifest"]["runtime_update_status"], "/update/status");
    assert_eq!(supply_chain["manifest"]["runtime_update_status_field"], "supply_chain");
}

#[test]
fn bundle_config_diagnostics_count_settings_policy_without_contents() {
    let _g = crate::lock_test_env();
    let home = fake_capsem_home();
    write(
        &home.path().join("settings.toml"),
        br#"
[profiles.rules.block_secret_host]
name = "block_secret_host"
action = "block"
match = 'http.host == "secret.example.invalid"'

[plugins.dummy_pre_eicar]
mode = "block"
"#,
    );

    let out = crate::support_bundle::run(None, 0, false, false).unwrap();
    let entries = read_tar_entries(&out);
    let diagnostics_entry = entries
        .iter()
        .find(|(p, _)| p.ends_with("system/config-diagnostics.json"))
        .expect("config diagnostics should be in bundle");
    let diagnostics: serde_json::Value = serde_json::from_slice(&diagnostics_entry.1).unwrap();
    assert_eq!(diagnostics["settings"]["ok"], true, "{diagnostics}");
    assert_eq!(diagnostics["settings"]["user_rule_count"], 1);
    assert_eq!(diagnostics["settings"]["plugin_count"], 1);
    assert!(diagnostics.get("profiles").is_none(), "{diagnostics}");
    assert!(
        !String::from_utf8_lossy(&diagnostics_entry.1).contains("secret.example.invalid"),
        "diagnostics carry counts, never rule contents"
    );
}

// ── Redaction over guest-influenced log bytes ──────────────────────
//
// serial.log is VM console output, so its bytes are the guest's to choose.
// Redaction used to decode the whole buffer and give up on the entire file if
// any of it was not UTF-8, which handed a guest a one-byte switch for turning
// off redaction of every credential in that log.

#[test]
fn one_invalid_byte_does_not_disable_redaction_for_the_whole_file() {
    let secret = "sk-ant-abcdefghijklmnopqrstuvwxyz";
    let mut log = format!("Authorization: Bearer {secret}\n").into_bytes();
    log.push(0xff); // a guest writing a single non-UTF-8 byte to its console

    let out = super::redact_log_bytes(&log);

    assert!(
        !out.windows(secret.len()).any(|w| w == secret.as_bytes()),
        "the credential survived: {}",
        String::from_utf8_lossy(&out)
    );
}

#[test]
fn undecodable_lines_pass_through_byte_exact() {
    // The original intent -- do not mangle binary -- still holds, just per
    // line instead of per file.
    let mut log = b"Authorization: Bearer sk-ant-abcdefghijklmnopqrstuvwxyz\n".to_vec();
    log.extend_from_slice(&[0xff, 0xfe, 0x00, 0x01]);
    log.push(b'\n');
    log.extend_from_slice(b"plain trailing line\n");

    let out = super::redact_log_bytes(&log);

    assert!(
        out.windows(4).any(|w| w == [0xff, 0xfe, 0x00, 0x01]),
        "binary line was altered"
    );
    assert!(out.ends_with(b"plain trailing line\n"), "tail line lost");
    assert!(!out.windows(3).any(|w| w == b"sk-"), "credential survived");
}

#[test]
fn redaction_preserves_line_structure_and_the_trailing_newline() {
    let log = b"one\ntwo\nthree\n".to_vec();
    assert_eq!(super::redact_log_bytes(&log), log);

    let no_trailing = b"one\ntwo".to_vec();
    assert_eq!(super::redact_log_bytes(&no_trailing), no_trailing);

    assert_eq!(super::redact_log_bytes(b""), b"");
}

#[test]
fn every_line_is_redacted_not_just_the_first() {
    let log = concat!(
        "boot ok\n",
        "Authorization: Bearer sk-ant-aaaaaaaaaaaaaaaaaaaaaaaa\n",
        "middle\n",
        "token ghp_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\n"
    )
    .as_bytes()
    .to_vec();

    let out = String::from_utf8(super::redact_log_bytes(&log)).unwrap();

    assert!(!out.contains("sk-ant-aaaaaaaaaaaaaaaaaaaaaaaa"), "{out}");
    assert!(!out.contains("ghp_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"), "{out}");
    assert!(out.contains("boot ok") && out.contains("middle"));
}

// ── Log tail reading ───────────────────────────────────────────────

#[test]
fn a_file_under_the_limit_is_returned_whole() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("small.log");
    fs::write(&path, b"line one\nline two\n").unwrap();

    assert_eq!(super::read_tail(&path, 1024).unwrap(), b"line one\nline two\n");
}

#[test]
fn an_oversized_file_returns_the_tail_starting_at_a_record_boundary() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("big.log");
    let mut body = String::new();
    for i in 0..500 {
        use std::fmt::Write as _;
        writeln!(body, "line {i:04}").unwrap();
    }
    fs::write(&path, &body).unwrap();

    let tail = super::read_tail(&path, 200).unwrap();

    assert!(tail.len() <= 200, "tail exceeded the cap: {}", tail.len());
    let text = String::from_utf8(tail).unwrap();
    assert!(
        text.starts_with("line "),
        "the partial leading record was not dropped: {text:?}"
    );
    assert!(text.ends_with("line 0499\n"), "tail is not the end of file");
}

#[test]
fn read_tail_reports_nothing_for_missing_paths_and_directories() {
    let dir = tempfile::tempdir().unwrap();

    assert_eq!(super::read_tail(&dir.path().join("absent.log"), 1024), None);
    assert_eq!(super::read_tail(dir.path(), 1024), None, "a directory is not a log");
}

#[test]
fn an_empty_file_reads_as_empty_rather_than_missing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("empty.log");
    fs::write(&path, b"").unwrap();

    assert_eq!(super::read_tail(&path, 1024), Some(Vec::new()));
}

#[test]
fn bundle_collects_every_rotated_file_in_a_log_stream() {
    // Rotation means today's `service.log` holds only today. A user reporting
    // "it broke on Tuesday" runs support-bundle on Thursday, so a bundle that
    // reads one filename ships the two days that do not contain the failure.
    let _g = crate::lock_test_env();
    let dir = fake_capsem_home();
    let run = dir.path().join("run");
    write(&run.join("service.2026-07-28.log"), b"INFO tuesday failure\n");
    write(&run.join("service.2026-07-29.log"), b"INFO wednesday\n");

    let out = crate::support_bundle::run(None, 0, false, false).unwrap();
    let entries = read_tar_entries(&out);

    // The rotated stream only -- raw stderr lives under host/stderr/ and is
    // covered by its own test.
    let logs: Vec<&String> = entries
        .iter()
        .map(|(p, _)| p)
        .filter(|p| p.contains("/host/service"))
        .collect();
    assert_eq!(
        logs.len(),
        3,
        "expected the whole service stream in the bundle, got {logs:?}"
    );

    let tuesday = entries
        .iter()
        .find(|(p, _)| p.ends_with("service.2026-07-28.log"))
        .expect("rotated file from the day of the failure is missing");
    assert!(String::from_utf8_lossy(&tuesday.1).contains("tuesday failure"));
}

#[test]
fn bundle_keeps_one_stream_out_of_another_streams_files() {
    let _g = crate::lock_test_env();
    let dir = fake_capsem_home();
    write(
        &dir.path().join("run").join("gateway.2026-07-29.log"),
        b"INFO gateway rotated\n",
    );

    let out = crate::support_bundle::run(None, 0, false, false).unwrap();
    let entries = read_tar_entries(&out);

    let service_entries: Vec<&String> = entries
        .iter()
        .map(|(p, _)| p)
        .filter(|p| p.contains("/host/service"))
        .collect();
    assert!(
        !service_entries.iter().any(|p| p.contains("gateway")),
        "gateway rotation leaked into the service stream: {service_entries:?}"
    );
}

#[test]
fn bundle_next_steps_name_files_the_bundle_actually_contains() {
    // The bundle ships its own reading instructions. They are the first thing
    // a maintainer follows and the last thing anyone remembers to update, so
    // every path they name -- literal or `<placeholder>` shaped -- must match
    // something really in the archive.
    let _g = crate::lock_test_env();
    let dir = fake_capsem_home();
    let run = dir.path().join("run");
    // The shape a live install has: rotated structured files, plus the
    // unrotated service.log that raw stderr (panics, pre-tracing death)
    // still lands in.
    write(&run.join("service.log"), b"thread 'main' panicked at src/main.rs\n");
    write(&run.join("service.2026-07-29.log"), b"INFO yesterday\n");
    // A session too, so the instructions that point into sessions/ are
    // checked against real entries rather than skipped for want of data.
    let session = dir.path().join("sessions").join("vm-abc");
    fs::create_dir_all(&session).unwrap();
    write(&session.join("process.log"), b"INFO process line\n");
    write(&session.join("serial.log"), b"[    0.0] boot\n");

    // Sessions included, as the default `capsem support-bundle` does.
    let out = crate::support_bundle::run(None, 3, false, false).unwrap();
    let entries = read_tar_entries(&out);
    let manifest: serde_json::Value = serde_json::from_slice(
        &entries
            .iter()
            .find(|(p, _)| p.ends_with("manifest.json"))
            .expect("manifest")
            .1,
    )
    .unwrap();

    let present: Vec<String> = manifest["sections"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| !s["missing"].as_bool().unwrap_or(false))
        .map(|s| s["path"].as_str().unwrap().to_string())
        .collect();

    for step in manifest["next_steps"].as_array().unwrap() {
        let step = step.as_str().unwrap();
        for token in step.split_whitespace() {
            let token = token.trim_matches(|c: char| !c.is_ascii_graphic() || c == '.');
            if !token.starts_with("host/") && !token.starts_with("sessions/") {
                continue;
            }
            // `<date>` / `<latest>` stand for a real segment; match on the
            // literal parts around them.
            let literal: Vec<&str> = token
                .split(['<', '>'])
                .step_by(2)
                .filter(|part| !part.is_empty())
                .collect();
            assert!(
                present.iter().any(|path| {
                    let mut rest = path.as_str();
                    literal.iter().all(|part| match rest.find(part) {
                        Some(at) => {
                            rest = &rest[at + part.len()..];
                            true
                        }
                        None => false,
                    })
                }),
                "next_steps sends the reader to {token:?}, which no section provides.\n\
                 sections: {present:#?}"
            );
        }
    }
}

#[test]
fn bundle_collects_raw_stderr_where_panics_land() {
    // A daemon that panicked wrote nothing through tracing -- the message is
    // in raw stderr and nowhere else. A bundle without it cannot explain the
    // single failure mode that produces no structured logs at all.
    let _g = crate::lock_test_env();
    let dir = fake_capsem_home();
    let stderr_dir = dir.path().join("run").join("stderr");
    fs::create_dir_all(&stderr_dir).unwrap();
    write(
        &stderr_dir.join("service.log"),
        b"thread 'main' panicked at crates/capsem-service/src/main.rs:42\n",
    );

    let out = crate::support_bundle::run(None, 3, false, false).unwrap();
    let entries = read_tar_entries(&out);

    let panic_entry = entries
        .iter()
        .find(|(p, _)| p.contains("stderr") && p.ends_with("service.log"))
        .expect("raw stderr missing from the bundle");
    assert!(String::from_utf8_lossy(&panic_entry.1).contains("panicked"));
}
