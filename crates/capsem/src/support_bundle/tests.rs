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

fn copy_dir_all(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap();
    for entry in fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let ty = entry.file_type().unwrap();
        let dst_path = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &dst_path);
        } else {
            fs::copy(entry.path(), dst_path).unwrap();
        }
    }
}

struct EnvVarGuard {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
        let previous = std::env::var_os(key);
        unsafe {
            std::env::set_var(key, value);
        }
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        unsafe {
            if let Some(previous) = &self.previous {
                std::env::set_var(self.key, previous);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }
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
    assert!(
        text.contains("<redacted>"),
        "redaction marker missing: {text}"
    );
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
    let _g = crate::lock_test_env();
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
fn bundle_includes_asset_manifest_origin_provenance() {
    let _g = crate::lock_test_env();
    let dir = fake_capsem_home();
    let home = dir.path();
    write(
        &home.join("assets/manifest.json"),
        br#"{"format":2,"refresh_policy":"24h","assets":{"current":"2026.0613.1","releases":{}},"binaries":{"current":"1.3.0","releases":{}}}"#,
    );
    write(
        &home.join("assets/manifest-origin.json"),
        br#"{"schema":"capsem.manifest_origin.v1","origin":"package","source":"file:///tmp/corp/manifest.json","packaged_at":"2026-06-13T00:00:00Z"}"#,
    );

    let out = crate::support_bundle::run(None, 0, false, false).unwrap();
    let entries = read_tar_entries(&out);

    let origin_entry = entries
        .iter()
        .find(|(p, _)| p.ends_with("assets/manifest-origin.json"))
        .expect("asset manifest origin provenance should be in support bundle");
    let origin: serde_json::Value = serde_json::from_slice(&origin_entry.1).unwrap();
    assert_eq!(origin["schema"], "capsem.manifest_origin.v1");
    assert_eq!(origin["origin"], "package");
    assert_eq!(origin["source"], "file:///tmp/corp/manifest.json");

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
                .is_some_and(|path| path.ends_with("assets/manifest-origin.json"))
                && section["missing"].as_bool() != Some(true)
                && section["kind"].as_str() == Some("json")
        }),
        "manifest-origin section missing from support manifest: {sections:#?}"
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
    let routes = boundary["debug_routes"].as_array().unwrap();
    for route in [
        "/profiles/{profile_id}/info",
        "/profiles/{profile_id}/obom",
        "/profiles/{profile_id}/assets/info",
        "/profiles/{profile_id}/plugins/info",
        "/profiles/{profile_id}/plugins/{plugin_id}/info",
        "/profiles/{profile_id}/plugins/credential_broker/credentials/info",
        "/profiles/{profile_id}/mcp/info",
        "/profiles/{profile_id}/mcp/default/info",
    ] {
        assert!(
            routes.iter().any(|candidate| candidate == route),
            "runtime boundary debug contract missing {route}: {boundary}"
        );
    }
    assert!(
        !routes
            .iter()
            .any(|route| route == "/profiles/{profile_id}/assets/status"),
        "runtime boundary debug contract must not advertise stale assets/status route: {boundary}"
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
    assert_eq!(
        supply_chain["host_sbom"]["release_artifact"],
        "capsem-sbom.spdx.json"
    );
    assert_eq!(supply_chain["host_sbom"]["scope"], "host_binaries");
    assert_eq!(
        supply_chain["host_sbom"]["attestation"],
        "github_attestations"
    );
    assert_eq!(
        supply_chain["profile_obom"]["runtime_routes"][0],
        "/profiles/{profile_id}/info"
    );
    assert_eq!(
        supply_chain["profile_obom"]["runtime_routes"][1],
        "/profiles/{profile_id}/obom"
    );
    assert_eq!(supply_chain["profile_obom"]["scope"], "base_image");
    assert_eq!(
        supply_chain["manifest"]["runtime_update_status"],
        "/update/status"
    );
    assert_eq!(
        supply_chain["manifest"]["runtime_update_status_field"],
        "supply_chain"
    );
}

#[test]
fn bundle_config_diagnostics_include_profile_obom_evidence() {
    use capsem_core::net::policy_config::current_profile_arch;

    let _g = crate::lock_test_env();
    let _home = fake_capsem_home();
    let profiles_dir = TempDir::new().unwrap();
    let profile_dir = profiles_dir.path().join("code");
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap();
    copy_dir_all(&repo_root.join("config/profiles/code"), &profile_dir);
    let obom_doc = br#"{"bomFormat":"CycloneDX","components":[{"name":"bash","version":"5.2"}]}"#;
    let obom_path = profile_dir.join("obom.cdx.json");
    write(&obom_path, obom_doc);
    let obom_hash = blake3::hash(obom_doc).to_hex().to_string();
    let arch = current_profile_arch().to_string();
    let mut profile_text = fs::read_to_string(profile_dir.join("profile.toml")).unwrap();
    profile_text.push_str(&format!(
        r#"

[obom]
format = "cyclonedx-obom.v1"

[obom.arch.{arch}]
name = "obom.cdx.json"
url = "file://{}"
hash = "blake3:{obom_hash}"
size = {}
generator = "cdxgen"
generator_version = "11.0.0"
"#,
        obom_path.display(),
        obom_doc.len()
    ));
    write(&profile_dir.join("profile.toml"), profile_text.as_bytes());
    let _profiles_guard = EnvVarGuard::set("CAPSEM_PROFILES_DIR", profiles_dir.path());

    let out = crate::support_bundle::run(None, 0, false, false).unwrap();
    let entries = read_tar_entries(&out);
    let diagnostics_entry = entries
        .iter()
        .find(|(p, _)| p.ends_with("system/config-diagnostics.json"))
        .expect("config diagnostics should be in bundle");
    let diagnostics: serde_json::Value = serde_json::from_slice(&diagnostics_entry.1).unwrap();
    let profile = diagnostics["profiles"]["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|profile| profile["id"] == "code")
        .expect("code profile should be in diagnostics");
    assert_eq!(profile["obom"]["current_arch"], arch);
    assert_eq!(profile["obom"]["hash"], format!("blake3:{obom_hash}"));
    assert_eq!(profile["obom"]["scope"], "base_image");
    assert_eq!(profile["obom"]["route"], "/profiles/code/obom");
}
