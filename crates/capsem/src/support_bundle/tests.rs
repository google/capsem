//! Tests for the support bundler. Use a fake `~/.capsem/` layout in a
//! tempdir, point CAPSEM_HOME at it, run `support_bundle::run`, and
//! inspect the emitted tar.gz.

use std::fs;
use std::io::Read;
use std::path::Path;
use std::sync::Mutex;
use tempfile::TempDir;

/// `CAPSEM_HOME` is a process-global env var; parallel test execution
/// would race on its value. Serialize every test that touches it.
static ENV_LOCK: Mutex<()> = Mutex::new(());

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
    // Required env so capsem_core::paths::capsem_home() points here.
    unsafe {
        std::env::set_var("CAPSEM_HOME", dir.path());
    }
    let home = dir.path();
    fs::create_dir_all(home.join("run")).unwrap();
    fs::create_dir_all(home.join("logs")).unwrap();
    fs::create_dir_all(home.join("sessions")).unwrap();
    write(
        &home.join("run/service.log"),
        b"INFO service line one\nINFO service line two\n",
    );
    write(&home.join("run/mcp.log"), b"INFO mcp starting\n");
    write(&home.join("run/gateway.pid"), b"12345");
    write(&home.join("run/gateway.port"), b"19222");
    write(
        &home.join("settings.toml"),
        br#"[provider.anthropic]
api_key = "sk-ant-real-secret-here-very-long-string"
endpoint = "https://api.anthropic.com"
"#,
    );
    write(
        &home.join("logs/20260502-180000.jsonl"),
        b"{\"level\":\"info\"}\n",
    );
    dir
}

#[test]
fn bundle_happy_path_writes_tar_gz_with_manifest() {
    let _g = ENV_LOCK.lock().unwrap();
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
    let _g = ENV_LOCK.lock().unwrap();
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
    assert!(
        text.contains("<redacted>"),
        "redaction marker missing: {text}"
    );
    // Non-secret value preserved:
    assert!(text.contains("https://api.anthropic.com"));
}

#[test]
fn bundle_no_redact_keeps_secrets() {
    let _g = ENV_LOCK.lock().unwrap();
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
    let _g = ENV_LOCK.lock().unwrap();
    let dir = fake_capsem_home();
    let home = dir.path();
    // Plant a gateway.token to make sure it's NOT in the bundle.
    write(
        &home.join("run/gateway.token"),
        b"tok-this-must-not-leak-abcd1234",
    );

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
    let _g = ENV_LOCK.lock().unwrap();
    let _dir = fake_capsem_home();
    // CAPSEM_HOME has no gateway.log, no tray.log -- expect missing entries.
    let out = crate::support_bundle::run(None, 0, false, false).unwrap();
    let entries = read_tar_entries(&out);

    let manifest_text = std::str::from_utf8(
        &entries
            .iter()
            .find(|(p, _)| p.ends_with("manifest.json"))
            .unwrap()
            .1,
    )
    .unwrap();
    let manifest: serde_json::Value = serde_json::from_str(manifest_text).unwrap();
    let sections = manifest["sections"].as_array().unwrap();
    let gateway_section = sections
        .iter()
        .find(|s| s["path"].as_str().unwrap_or("").ends_with("gateway.log"))
        .expect("gateway.log section missing");
    assert_eq!(gateway_section["missing"], true);
}

#[test]
fn bundle_includes_runtime_boundary_debug_contract() {
    let _g = ENV_LOCK.lock().unwrap();
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
        services
            .iter()
            .any(|s| s["service"] == "audit" && s["port"] == 5006),
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
}
