use super::*;

#[test]
fn service_settings_defaults_validate() {
    let settings = ServiceSettings::default();
    settings.validate().unwrap();
    assert_eq!(settings.profiles.default_profile, EVERYDAY_WORK_PROFILE_ID);
    assert!(!settings.telemetry.enabled);
    assert!(!settings.remote_policy.enabled);
    assert!(!settings.profile_catalog.is_configured());
    assert_eq!(settings.profile_catalog.check_interval_secs, 21_600);
    assert_eq!(
        settings.profiles.user_dirs,
        vec![crate::paths::capsem_home().join("profiles")]
    );
}

#[test]
fn service_settings_parse_toml_with_plugins_and_credentials() {
    let settings = ServiceSettings::from_toml_str(
        r#"
version = 1

[profiles]
base_dirs = ["/opt/capsem/profiles/base"]
corp_dirs = ["/opt/capsem/profiles/corp"]
user_dirs = ["/Users/test/.capsem/profiles"]
default_profile = "everyday-work"
allow_user_profiles = true
allow_user_fork = true
allow_user_delete = false

[assets]
assets_dir = "/opt/capsem/assets"
image_roots = ["/opt/capsem/images", "/Users/test/.capsem/images"]
download_base_url = "https://assets.example.test/capsem"

[credentials]
backend = "toml"

[credentials.items.openai]
description = "OpenAI API key"
value = "sk-test"

[telemetry]
enabled = true
endpoint = "https://otel.example.test/v1/traces"
batch_max_events = 64
flush_interval_ms = 1000

[remote_policy]
enabled = true
endpoint = "https://policy.example.test/decision"
auth_token = "test-token"
timeout_ms = 2000
failure_mode = "fail-closed"

[profile_catalog]
manifest_url = "https://profiles.example.test/catalog.json"
profile_payload_pubkey = "untrusted comment: profile payload test key"
check_interval_secs = 300
"#,
    )
    .unwrap();

    assert_eq!(settings.credentials.items["openai"].value, "sk-test");
    assert_eq!(
        settings.telemetry.endpoint.as_deref(),
        Some("https://otel.example.test/v1/traces")
    );
    assert_eq!(settings.remote_policy.timeout_ms, 2000);
    assert_eq!(
        settings.assets.download_base_url.as_deref(),
        Some("https://assets.example.test/capsem")
    );
    assert_eq!(
        settings.profile_catalog.manifest_url.as_deref(),
        Some("https://profiles.example.test/catalog.json")
    );
    assert_eq!(
        settings.profile_catalog.profile_payload_pubkey.as_deref(),
        Some("untrusted comment: profile payload test key")
    );
    assert_eq!(settings.profile_catalog.check_interval_secs, 300);
}

#[test]
fn service_settings_reject_profile_catalog_without_pubkey() {
    let error = ServiceSettings::from_toml_str(
        r#"
[profile_catalog]
manifest_url = "https://profiles.example.test/catalog.json"
"#,
    )
    .unwrap_err();

    assert!(error
        .to_string()
        .contains("profile_catalog.profile_payload_pubkey"));
}

#[test]
fn service_settings_reject_profile_catalog_non_loopback_http() {
    let error = ServiceSettings::from_toml_str(
        r#"
[profile_catalog]
manifest_url = "http://profiles.example.test/catalog.json"
profile_payload_pubkey = "untrusted comment: profile payload test key"
"#,
    )
    .unwrap_err();

    assert!(error.to_string().contains("profile_catalog.manifest_url"));
    assert!(error.to_string().contains("must use https://"));
}

#[test]
fn service_settings_accept_profile_catalog_loopback_http_for_dev() {
    let settings = ServiceSettings::from_toml_str(
        r#"
[profile_catalog]
manifest_url = "http://127.0.0.1:8080/catalog.json"
profile_payload_pubkey = "untrusted comment: profile payload test key"
"#,
    )
    .unwrap();

    assert!(settings.profile_catalog.is_configured());
}

#[test]
fn service_settings_reject_enabled_plugin_without_endpoint() {
    let error = ServiceSettings::from_toml_str(
        r#"
[telemetry]
enabled = true
"#,
    )
    .unwrap_err();

    assert!(error.to_string().contains("telemetry.endpoint"));
}

#[test]
fn service_settings_reject_unknown_fields() {
    let error = ServiceSettings::from_toml_str(
        r#"
version = 1
legacy_policy = true
"#,
    )
    .unwrap_err();

    assert!(error.to_string().contains("unknown field"));
}

#[test]
fn service_settings_reject_malformed_toml() {
    let error = ServiceSettings::from_toml_str(
        r#"
[telemetry
enabled = true
"#,
    )
    .unwrap_err();

    assert!(matches!(error, SettingsProfilesError::Parse { .. }));
}

#[test]
fn service_settings_reject_invalid_plugin_endpoint_scheme() {
    let error = ServiceSettings::from_toml_str(
        r#"
[remote_policy]
enabled = true
endpoint = "ftp://policy.example.test/decision"
"#,
    )
    .unwrap_err();

    assert!(error.to_string().contains("remote_policy.endpoint"));
    assert!(error.to_string().contains("http:// or https://"));
}

#[test]
fn service_settings_reject_empty_credential_value() {
    let error = ServiceSettings::from_toml_str(
        r#"
[credentials.items.openai]
value = "   "
"#,
    )
    .unwrap_err();

    assert!(error.to_string().contains("credentials.items.openai.value"));
}

#[test]
fn service_settings_accept_custom_image_roots() {
    let settings = ServiceSettings::from_toml_str(
        r#"
[profiles]
base_dirs = ["/opt/capsem/profiles/base"]

[assets]
assets_dir = "/opt/capsem/assets"
image_roots = ["/opt/capsem/images"]
"#,
    )
    .unwrap();

    assert_eq!(
        settings.assets.image_roots,
        vec![PathBuf::from("/opt/capsem/images")]
    );
}

#[test]
fn service_settings_rejects_legacy_manifest_settings() {
    let error = ServiceSettings::from_toml_str(
        r#"
[assets.manifest]
source = "remote-url"
"#,
    )
    .unwrap_err();

    assert!(error.to_string().contains("unknown field"));
}

#[test]
fn service_settings_reject_invalid_asset_download_endpoint() {
    let error = ServiceSettings::from_toml_str(
        r#"
[assets]
download_base_url = "file:///tmp/assets"
"#,
    )
    .unwrap_err();

    assert!(error.to_string().contains("assets.download_base_url"));
}

#[test]
fn service_asset_resolution_uses_service_assets_dir_without_cli_override() {
    let mut settings = ServiceSettings::default();
    settings.assets.assets_dir = Some(PathBuf::from("/corp/capsem/assets"));

    let resolved = resolve_service_asset_locations(
        &settings,
        None,
        Some(PathBuf::from("/installed/capsem/assets")),
        PathBuf::from("assets"),
    )
    .unwrap();

    assert_eq!(resolved.assets_dir, PathBuf::from("/corp/capsem/assets"));
    assert_eq!(
        resolved.assets_dir_origin,
        ServiceSettingOrigin::ServiceSettings
    );
}

#[test]
fn service_asset_resolution_prefers_cli_assets_dir_over_service_settings() {
    let mut settings = ServiceSettings::default();
    settings.assets.assets_dir = Some(PathBuf::from("/corp/capsem/assets"));

    let resolved = resolve_service_asset_locations(
        &settings,
        Some(PathBuf::from("/cli/capsem/assets")),
        Some(PathBuf::from("/installed/capsem/assets")),
        PathBuf::from("assets"),
    )
    .unwrap();

    assert_eq!(resolved.assets_dir, PathBuf::from("/cli/capsem/assets"));
    assert_eq!(resolved.assets_dir_origin, ServiceSettingOrigin::Cli);
}

#[test]
fn service_asset_resolution_preserves_image_roots_and_download_endpoint() {
    let settings = ServiceSettings::from_toml_str(
        r#"
[assets]
assets_dir = "/corp/capsem/assets"
image_roots = ["/corp/capsem/images", "/shared/capsem/images"]
download_base_url = "https://assets.example.test/capsem"
"#,
    )
    .unwrap();

    let resolved =
        resolve_service_asset_locations(&settings, None, None, PathBuf::from("assets")).unwrap();

    assert_eq!(resolved.assets_dir, PathBuf::from("/corp/capsem/assets"));
    assert_eq!(
        resolved.image_roots,
        vec![
            PathBuf::from("/corp/capsem/images"),
            PathBuf::from("/shared/capsem/images")
        ]
    );
    assert_eq!(
        resolved.image_roots_origin,
        ServiceSettingOrigin::ServiceSettings
    );
    assert_eq!(
        resolved.download_base_url.as_deref(),
        Some("https://assets.example.test/capsem")
    );
}

#[test]
fn service_settings_file_round_trip_creates_parent_dirs() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("nested").join("service.toml");
    let mut settings = ServiceSettings::default();
    settings.profiles.base_dirs = vec![temp.path().join("profiles").join("base")];
    settings.profiles.user_dirs = vec![temp.path().join("profiles").join("user")];
    settings.telemetry.enabled = true;
    settings.telemetry.endpoint = Some("https://otel.example.test/v1/traces".to_string());

    write_service_settings(&path, &settings).unwrap();
    let loaded = load_service_settings(&path).unwrap();

    assert_eq!(loaded, settings);
}

#[test]
fn service_settings_load_or_default_returns_default_for_missing_file() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("missing").join("service.toml");

    let settings = load_service_settings_or_default(&path).unwrap();

    assert_eq!(settings, ServiceSettings::default());
}

#[test]
fn service_settings_file_load_rejects_unknown_fields() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("service.toml");
    fs::write(
        &path,
        r#"
version = 1
settings = "v1"
"#,
    )
    .unwrap();

    let error = load_service_settings(&path).unwrap_err();

    assert!(error.to_string().contains("unknown field"));
}

#[test]
fn service_settings_file_write_rejects_invalid_settings() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("service.toml");
    let mut settings = ServiceSettings::default();
    settings.telemetry.enabled = true;

    let error = write_service_settings(&path, &settings).unwrap_err();

    assert!(error.to_string().contains("telemetry.endpoint"));
    assert!(!path.exists());
}

#[test]
fn everyday_work_profile_has_default_icon_and_security_capabilities() {
    let profile = Profile::everyday_work();
    profile.validate().unwrap();
    assert_eq!(profile.id, EVERYDAY_WORK_PROFILE_ID);
    assert_eq!(profile.profile_type, ProfileType::EverydayWork);
    assert!(profile.icon_svg_or_default().contains("<svg"));
    assert_eq!(
        profile.security.capabilities.credential_brokerage,
        CapabilityMode::Ask
    );
}

#[test]
fn profile_parse_toml_with_profile_scoped_sections() {
    let profile = Profile::from_toml_str(
        r#"
version = 1
id = "coding"
name = "For Coding"
description = "Technical default profile."
best_for = "Coding sessions with repository tools."
profile_type = "coding"
extends_profile_id = "everyday-work"
icon_svg = "<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>"

[appearance]
theme = "inherit-service"
accent = "green"

[ai.providers.openai]
enabled = true
model = "gpt-5.2"
base_url = "https://api.openai.com/v1"
credential_refs = ["openai"]

[mcpServers.github]
enabled = true
type = "stdio"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-github"]
[mcpServers.github.env]
GITHUB_TOKEN = "env:CAPSEM_GITHUB_TOKEN"
[mcpServers.github.capsem]
credential_refs = ["github"]
allowed_tools = ["repo.read", "issue.write"]

[skills]
groups = ["dev"]
enabled = ["dev-sprint"]

[vm]
memory_mib = 8192
cpus = 6
network = "proxied"
track_rootfs_dependencies = true

[security.capabilities]
credential_brokerage = "ask"
pii_detection = "ask"
mcp_rag = "ask"
mcp_tools = "ask"
network_egress = "ask"
file_boundaries = "ask"
audit = "audit"

[security.rules.http.block-secret-egress]
on = "http.request"
if = "request.data.contains_secret"
decision = "block"
reason = "Secrets must not leave the VM."
"#,
    )
    .unwrap();

    assert_eq!(profile.id, "coding");
    assert_eq!(profile.extends_profile_id.as_deref(), Some("everyday-work"));
    assert_eq!(
        profile.ai.providers["openai"].model.as_deref(),
        Some("gpt-5.2")
    );
    let github = &profile.mcp.connectors["github"];
    assert_eq!(github.server_type.as_deref(), Some("stdio"));
    assert_eq!(github.command.as_deref(), Some("npx"));
    assert_eq!(
        github.args,
        vec![
            "-y".to_string(),
            "@modelcontextprotocol/server-github".to_string()
        ]
    );
    assert_eq!(
        github.env.get("GITHUB_TOKEN").map(String::as_str),
        Some("env:CAPSEM_GITHUB_TOKEN")
    );
    assert_eq!(github.capsem.allowed_tools.len(), 2);
    let rule = &profile.security.rules.http["block-secret-egress"];
    assert_eq!(rule.callback, "http.request");
    assert_eq!(rule.condition, "request.data.contains_secret");
    assert_eq!(rule.priority, 1);
}

#[test]
fn profile_parse_rejects_legacy_mcp_connectors_shape() {
    let err = Profile::from_toml_str(
        r#"
version = 1
id = "coding"
name = "For Coding"
best_for = "Coding sessions with repository tools."
profile_type = "coding"

[mcp.connectors.github]
enabled = true
allowed_tools = ["repo.read"]
"#,
    )
    .unwrap_err();

    assert!(
        err.to_string().contains("unknown field `mcp`"),
        "unexpected error: {err}"
    );
}

#[test]
fn profile_parse_toml_with_package_tool_and_asset_contracts() {
    let profile = Profile::from_toml_str(
        r#"
version = 1
id = "coding"
name = "For Coding"
description = "Technical default profile."
best_for = "Coding sessions with repository tools."
profile_type = "coding"

[packages.runtimes]
python = "3.12.3"
node = "22.1.0"
uv = "0.4.30"

[packages.python_modules]
requests = "2.32.3"
numpy = "1.26.4"

[packages.node_packages]
"@modelcontextprotocol/sdk" = "1.2.3"
playwright = "1.44.0"

[packages.system]
distro = "debian"
release = "bookworm"

[packages.system.apt]
curl = "8.11.1-1"
ca-certificates = "20240203"

[tools.capsem_doctor]
version = "2026.05.18"
required = true
source = "guest"

[tools.uv]
version = "0.4.30"
required = true
source = "guest"

[vm.assets.arm64.kernel]
url = "https://assets.capsem.dev/profiles/coding/2026.0520.1/arm64/vmlinuz"
hash = "blake3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
signature_url = "https://assets.capsem.dev/profiles/coding/2026.0520.1/arm64/vmlinuz.minisig"
size = 7797248
content_type = "application/octet-stream"

[vm.assets.arm64.initrd]
url = "https://assets.capsem.dev/profiles/coding/2026.0520.1/arm64/initrd.img"
hash = "blake3:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
signature_url = "https://assets.capsem.dev/profiles/coding/2026.0520.1/arm64/initrd.img.minisig"
size = 2270154
content_type = "application/octet-stream"

[vm.assets.arm64.rootfs]
url = "https://assets.capsem.dev/profiles/coding/2026.0520.1/arm64/rootfs.squashfs"
hash = "blake3:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
signature_url = "https://assets.capsem.dev/profiles/coding/2026.0520.1/arm64/rootfs.squashfs.minisig"
size = 454230016
content_type = "application/vnd.squashfs"
"#,
    )
    .unwrap();

    assert_eq!(profile.packages.runtimes["python"], "3.12.3");
    assert_eq!(
        profile.packages.node_packages["@modelcontextprotocol/sdk"],
        "1.2.3"
    );
    assert_eq!(profile.packages.system.distro, "debian");
    assert_eq!(
        profile.tools["capsem_doctor"].source,
        ProfileToolSource::Guest
    );

    let arm64 = &profile.vm.assets["arm64"];
    assert_eq!(arm64.kernel.size, 7_797_248);
    assert_eq!(
        arm64.initrd.hash.as_str(),
        "blake3:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
    );
    assert_eq!(arm64.rootfs.content_type, "application/vnd.squashfs");
}

#[test]
fn profile_rejects_asset_hashes_that_are_not_canonical_blake3() {
    let error = Profile::from_toml_str(
        r#"
version = 1
id = "coding"
name = "For Coding"
best_for = "Coding."

[vm.assets.arm64.kernel]
url = "https://assets.capsem.dev/kernel"
hash = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
signature_url = "https://assets.capsem.dev/kernel.minisig"
size = 1
content_type = "application/octet-stream"

[vm.assets.arm64.initrd]
url = "https://assets.capsem.dev/initrd"
hash = "blake3:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
signature_url = "https://assets.capsem.dev/initrd.minisig"
size = 1
content_type = "application/octet-stream"

[vm.assets.arm64.rootfs]
url = "https://assets.capsem.dev/rootfs"
hash = "blake3:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
signature_url = "https://assets.capsem.dev/rootfs.minisig"
size = 1
content_type = "application/vnd.squashfs"
"#,
    )
    .unwrap_err();

    assert!(error.to_string().contains("canonical blake3"));
}

#[test]
fn profile_rejects_asset_locations_with_path_traversal() {
    let error = Profile::from_toml_str(
        r#"
version = 1
id = "coding"
name = "For Coding"
best_for = "Coding."

[vm.assets.arm64.kernel]
url = "file:///tmp/capsem/../kernel"
hash = "blake3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
signature_url = "file:///tmp/capsem/kernel.minisig"
size = 1
content_type = "application/octet-stream"

[vm.assets.arm64.initrd]
url = "file:///tmp/capsem/initrd"
hash = "blake3:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
signature_url = "file:///tmp/capsem/initrd.minisig"
size = 1
content_type = "application/octet-stream"

[vm.assets.arm64.rootfs]
url = "file:///tmp/capsem/rootfs"
hash = "blake3:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
signature_url = "file:///tmp/capsem/rootfs.minisig"
size = 1
content_type = "application/vnd.squashfs"
"#,
    )
    .unwrap_err();

    assert!(error.to_string().contains("path traversal"));
}

#[test]
fn profile_rejects_tool_contract_without_version() {
    let error = Profile::from_toml_str(
        r#"
version = 1
id = "coding"
name = "For Coding"
best_for = "Coding."

[tools.capsem_doctor]
required = true
source = "guest"
"#,
    )
    .unwrap_err();

    assert!(error.to_string().contains("missing field `version`"));
}

#[test]
fn profile_rejects_invalid_rule_names() {
    let error = Profile::from_toml_str(
        r#"
id = "coding"
name = "Coding"
best_for = "Coding"

[security.rules.http."bad*name"]
on = "http.request"
if = "true"
decision = "block"
"#,
    )
    .unwrap_err();

    assert!(error.to_string().contains("rule name"));
}

#[test]
fn profile_rejects_rule_callback_type_mismatch() {
    let error = Profile::from_toml_str(
        r#"
id = "coding"
name = "Coding"
best_for = "Coding"

[security.rules.http.not-http]
on = "mcp.request"
if = "true"
decision = "block"
"#,
    )
    .unwrap_err();

    assert!(error
        .to_string()
        .contains("not allowed for rule type 'http'"));
}

#[test]
fn profile_rejects_legacy_dns_query_callback() {
    let error = Profile::from_toml_str(
        r#"
id = "coding"
name = "Coding"
best_for = "Coding"

[security.rules.dns.deny-exfil]
on = "dns.query"
if = "true"
decision = "block"
"#,
    )
    .unwrap_err();

    let message = error.to_string();
    assert!(
        message.contains("was renamed to 'dns.request'"),
        "expected rename hint, got: {message}"
    );
}

#[test]
fn profile_accepts_mcp_arguments_dotted_paths() {
    let profile = Profile::from_toml_str(
        r#"
id = "coding"
name = "Coding"
best_for = "Coding"

[security.rules.mcp.redact-issue-title]
on = "mcp.request"
if = 'method == "tools/call" && arguments.issue.title.contains("prod-token-")'
decision = "rewrite"
rewrite_target = 'arguments.issue.title =~ "(?P<prefix>prod-token-)[A-Za-z0-9]+"'
rewrite_value = "${prefix}[redacted]"
"#,
    )
    .unwrap();

    let rule = &profile.security.rules.mcp["redact-issue-title"];
    assert_eq!(rule.callback, "mcp.request");
    assert!(rule.condition.contains("arguments.issue.title"));
    assert_eq!(
        rule.rewrite_target.as_deref(),
        Some(r#"arguments.issue.title =~ "(?P<prefix>prod-token-)[A-Za-z0-9]+""#)
    );
    assert_eq!(rule.rewrite_value.as_deref(), Some("${prefix}[redacted]"));
}

#[test]
fn profile_accepts_rewrite_rule_with_captures() {
    let profile = Profile::from_toml_str(
        r#"
id = "coding"
name = "Coding"
best_for = "Coding"

[security.rules.http.rewrite-openai]
on = "http.request"
if = "true"
decision = "rewrite"
rewrite_target = 'request.url =~ "^https://github\.com/openai/(?P<repo>[^/?#]+)$"'
rewrite_value = "https://github.com/openclaw/${repo}"
"#,
    )
    .unwrap();

    let rule = &profile.security.rules.http["rewrite-openai"];
    assert_eq!(rule.decision, RuleDecision::Rewrite);
    assert_eq!(
        rule.rewrite_target.as_deref(),
        Some(r#"request.url =~ "^https://github\.com/openai/(?P<repo>[^/?#]+)$""#)
    );
    assert_eq!(
        rule.rewrite_value.as_deref(),
        Some("https://github.com/openclaw/${repo}")
    );
}

#[test]
fn profile_rejects_rewrite_rule_missing_fields() {
    let error = Profile::from_toml_str(
        r#"
id = "coding"
name = "Coding"
best_for = "Coding"

[security.rules.http.rewrite-openai]
on = "http.request"
if = "true"
decision = "rewrite"
"#,
    )
    .unwrap_err();

    assert!(error
        .to_string()
        .contains("rewrite decisions require rewrite_target and rewrite_value"));
}

#[test]
fn profile_rejects_rewrite_value_with_unknown_capture() {
    let error = Profile::from_toml_str(
        r#"
id = "coding"
name = "Coding"
best_for = "Coding"

[security.rules.http.rewrite-openai]
on = "http.request"
if = "true"
decision = "rewrite"
rewrite_target = 'request.url =~ "^https://github\.com/openai/(?P<repo>[^/?#]+)$"'
rewrite_value = "https://github.com/openclaw/${missing}"
"#,
    )
    .unwrap_err();

    assert!(error
        .to_string()
        .contains("rewrite_value references unknown capture 'missing'"));
}

#[test]
fn profile_rejects_rewrite_fields_for_non_rewrite_decision() {
    let error = Profile::from_toml_str(
        r#"
id = "coding"
name = "Coding"
best_for = "Coding"

[security.rules.http.block-openai]
on = "http.request"
if = "true"
decision = "block"
rewrite_target = 'request.url =~ "^https://github\.com/openai/.+$"'
rewrite_value = "https://github.com/openclaw/repo"
"#,
    )
    .unwrap_err();

    assert!(error
        .to_string()
        .contains("only rewrite decisions may include rewrite_target/rewrite_value"));
}

#[test]
fn profile_rejects_bad_profile_id() {
    let error = Profile::from_toml_str(
        r#"
id = "../escape"
name = "Bad"
best_for = "Bad"
"#,
    )
    .unwrap_err();

    assert!(error.to_string().contains("profile id"));
}

#[test]
fn profile_rejects_legacy_profile_type_values() {
    let error = Profile::from_toml_str(
        r#"
id = "legacy"
name = "Legacy"
best_for = "Legacy"
profile_type = "research"
"#,
    )
    .unwrap_err();

    assert!(error.to_string().contains("unknown variant"));
    assert!(error.to_string().contains("research"));
}

#[test]
fn profile_rejects_invalid_extends_profile_id() {
    let error = Profile::from_toml_str(
        r#"
id = "coding"
name = "Coding"
best_for = "Coding"
extends_profile_id = "../bad-parent"
"#,
    )
    .unwrap_err();

    assert!(error.to_string().contains("extends_profile_id"));
}

#[test]
fn profile_rejects_self_referential_extends_profile_id() {
    let error = Profile::from_toml_str(
        r#"
id = "coding"
name = "Coding"
best_for = "Coding"
extends_profile_id = "coding"
"#,
    )
    .unwrap_err();

    assert!(error
        .to_string()
        .contains("cannot reference the profile itself"));
}

#[test]
fn profile_rejects_non_svg_icon() {
    let error = Profile::from_toml_str(
        r#"
id = "bad-icon"
name = "Bad Icon"
best_for = "Bad Icon"
icon_svg = "<script>alert(1)</script>"
"#,
    )
    .unwrap_err();

    assert!(error.to_string().contains("icon must be inline SVG"));
}

#[test]
fn profile_rejects_duplicate_enabled_skills() {
    let error = Profile::from_toml_str(
        r#"
id = "skills"
name = "Skills"
best_for = "Skills"

[skills]
enabled = ["dev-sprint", "dev-sprint"]
"#,
    )
    .unwrap_err();

    assert!(error.to_string().contains("duplicate id 'dev-sprint'"));
}

#[test]
fn profile_rejects_bad_connector_credential_ref() {
    let error = Profile::from_toml_str(
        r#"
id = "connector"
name = "Connector"
best_for = "Connector"

[mcpServers.github]
enabled = true
command = "npx"
[mcpServers.github.capsem]
credential_refs = ["../github-token"]
"#,
    )
    .unwrap_err();

    assert!(error.to_string().contains("credential_refs"));
}

#[test]
fn profile_discovery_reads_builtin_and_profile_dirs() {
    let temp = tempfile::tempdir().unwrap();
    let base_dir = temp.path().join("base");
    let user_dir = temp.path().join("user");
    fs::create_dir_all(&base_dir).unwrap();
    fs::create_dir_all(&user_dir).unwrap();
    fs::write(
        base_dir.join("coding.toml"),
        profile_toml("coding", "For Coding", "coding"),
    )
    .unwrap();

    let roots = test_roots(base_dir, user_dir);
    let catalog = discover_profiles(&roots).unwrap();

    let everyday = catalog.get(EVERYDAY_WORK_PROFILE_ID).unwrap();
    assert_eq!(everyday.source, ProfileSource::BuiltIn);
    assert!(everyday.locked);

    let coding = catalog.get("coding").unwrap();
    assert_eq!(coding.source, ProfileSource::Base);
    assert!(coding.locked);
    assert_eq!(coding.profile.profile_type, ProfileType::Coding);
}

#[test]
fn profile_discovery_rejects_duplicate_file_ids() {
    let temp = tempfile::tempdir().unwrap();
    let base_dir = temp.path().join("base");
    let corp_dir = temp.path().join("corp");
    let user_dir = temp.path().join("user");
    fs::create_dir_all(&base_dir).unwrap();
    fs::create_dir_all(&corp_dir).unwrap();
    fs::write(
        base_dir.join("coding.toml"),
        profile_toml("coding", "Base Coding", "coding"),
    )
    .unwrap();
    fs::write(
        corp_dir.join("coding.toml"),
        profile_toml("coding", "Corp Coding", "coding"),
    )
    .unwrap();

    let mut roots = test_roots(base_dir, user_dir);
    roots.corp_dirs = vec![corp_dir];
    let error = discover_profiles(&roots).unwrap_err();

    assert!(matches!(
        error,
        SettingsProfilesError::DuplicateProfile { .. }
    ));
}

#[test]
fn user_profile_create_update_delete_round_trip() {
    let temp = tempfile::tempdir().unwrap();
    let base_dir = temp.path().join("base");
    let user_dir = temp.path().join("user");
    fs::create_dir_all(&base_dir).unwrap();
    let roots = test_roots(base_dir, user_dir.clone());

    let created = create_user_profile(
        &roots,
        profile_value("custom", "Custom", ProfileType::Coding),
    )
    .unwrap();
    assert_eq!(created.source, ProfileSource::User);
    assert!(!created.locked);
    assert!(user_dir.join("custom.toml").exists());

    let mut updated = created.profile.clone();
    updated.name = "Custom Updated".to_string();
    update_user_profile(&roots, updated).unwrap();

    let catalog = discover_profiles(&roots).unwrap();
    assert_eq!(
        catalog.get("custom").unwrap().profile.name,
        "Custom Updated"
    );

    delete_user_profile(&roots, "custom").unwrap();
    let catalog = discover_profiles(&roots).unwrap();
    assert!(catalog.get("custom").is_none());
}

#[test]
fn profile_payload_v2_converts_to_runtime_profile_shape() {
    let payload = include_str!("../../../../schemas/fixtures/profile-v2-valid.json");
    let value = crate::profile_payload_schema::validate_profile_payload_v2_json(payload).unwrap();

    let profile = Profile::from_profile_payload_v2_value(value).unwrap();

    assert_eq!(profile.version, SETTINGS_SCHEMA_VERSION);
    assert_eq!(profile.id, EVERYDAY_WORK_PROFILE_ID);
    assert_eq!(profile.packages.runtimes["python"], "3.12.3");
    assert_eq!(profile.vm.memory_mib, 8192);
    assert_eq!(
        profile.vm.assets["arm64"].rootfs.hash,
        "blake3:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
    );
    assert_eq!(
        profile.security.rules.http["allow-api"].callback,
        "http.request"
    );

    let toml = toml::to_string_pretty(&profile).unwrap();
    let reparsed = Profile::from_toml_str(&toml).unwrap();
    assert_eq!(reparsed.id, profile.id);
    assert_eq!(reparsed.vm.assets, profile.vm.assets);
}

#[test]
fn install_verified_profile_payload_materializes_runtime_profile_and_revision_payload() {
    let temp = tempfile::tempdir().unwrap();
    let base_dir = temp.path().join("base");
    let corp_dir = temp.path().join("corp");
    let user_dir = temp.path().join("user");
    fs::create_dir_all(&base_dir).unwrap();
    let mut roots = test_roots(base_dir, user_dir);
    roots.corp_dirs = vec![corp_dir.clone()];

    let payload = include_str!("../../../../schemas/fixtures/profile-v2-valid.json");
    let profile_hash = format!("blake3:{}", blake3::hash(payload.as_bytes()).to_hex());
    let manifest = crate::profile_manifest::ProfileManifest::from_json(&format!(
        r#"{{
          "format": 1,
          "profiles": {{
            "everyday-work": {{
              "current_revision": "2026.0520.1",
              "revisions": {{
                "2026.0520.1": {{
                  "status": "active",
                  "min_binary": "1.0.0",
                  "profile_url": "https://assets.capsem.dev/profile.json",
                  "profile_hash": "{profile_hash}",
                  "profile_signature_url": "https://assets.capsem.dev/profile.json.minisig"
                }}
              }}
            }}
          }}
        }}"#
    ))
    .unwrap();
    let revision = manifest.revision("everyday-work", "2026.0520.1").unwrap();
    let verified =
        crate::profile_manifest::verify_installable_profile_payload(revision, payload).unwrap();

    let installed = install_verified_profile_payload(&roots, &verified).unwrap();

    assert_eq!(installed.profile_id, EVERYDAY_WORK_PROFILE_ID);
    assert_eq!(installed.revision, "2026.0520.1");
    assert_eq!(installed.payload_hash, profile_hash);
    assert_eq!(
        installed.runtime_profile_path,
        corp_dir.join("everyday-work.toml")
    );
    assert!(installed.runtime_profile_path.exists());
    assert_eq!(
        installed.payload_path,
        corp_dir
            .join(".catalog")
            .join("profiles")
            .join("everyday-work")
            .join("2026.0520.1")
            .join("profile.json")
    );
    assert!(installed.payload_path.exists());
    assert_eq!(
        installed.current_record_path,
        corp_dir
            .join(".catalog")
            .join("profiles")
            .join("everyday-work")
            .join("current.json")
    );
    assert!(installed.current_record_path.exists());
    let installed_payload = fs::read_to_string(&installed.payload_path).unwrap();
    assert_eq!(
        format!(
            "blake3:{}",
            blake3::hash(installed_payload.as_bytes()).to_hex()
        ),
        profile_hash
    );
    let current = load_installed_profile_revision(&roots, EVERYDAY_WORK_PROFILE_ID)
        .unwrap()
        .expect("current installed revision should be recorded");
    assert_eq!(current.profile_id, EVERYDAY_WORK_PROFILE_ID);
    assert_eq!(current.revision, "2026.0520.1");
    assert_eq!(current.payload_hash, profile_hash);
    let complete = load_complete_installed_profile_revision(&roots, EVERYDAY_WORK_PROFILE_ID)
        .unwrap()
        .expect("complete installed revision should include runtime and payload files");
    assert_eq!(complete.profile_id, EVERYDAY_WORK_PROFILE_ID);
    assert_eq!(complete.revision, "2026.0520.1");
    assert_eq!(complete.payload_hash, profile_hash);
    assert_eq!(
        complete.runtime_profile_path,
        installed.runtime_profile_path
    );
    assert_eq!(complete.payload_path, installed.payload_path);

    let catalog = discover_profiles(&roots).unwrap();
    let record = catalog.get(EVERYDAY_WORK_PROFILE_ID).unwrap();
    assert_eq!(record.source, ProfileSource::Corp);
    assert!(record.locked);
    assert_eq!(record.profile.packages.runtimes["python"], "3.12.3");
}

#[test]
fn load_complete_installed_profile_revision_rejects_payload_hash_drift() {
    let temp = tempfile::tempdir().unwrap();
    let base_dir = temp.path().join("base");
    let corp_dir = temp.path().join("corp");
    let user_dir = temp.path().join("user");
    fs::create_dir_all(&base_dir).unwrap();
    let mut roots = test_roots(base_dir, user_dir);
    roots.corp_dirs = vec![corp_dir.clone()];

    let payload = include_str!("../../../../schemas/fixtures/profile-v2-valid.json");
    let profile_hash = format!("blake3:{}", blake3::hash(payload.as_bytes()).to_hex());
    let manifest = crate::profile_manifest::ProfileManifest::from_json(&format!(
        r#"{{
          "format": 1,
          "profiles": {{
            "everyday-work": {{
              "current_revision": "2026.0520.1",
              "revisions": {{
                "2026.0520.1": {{
                  "status": "active",
                  "min_binary": "1.0.0",
                  "profile_url": "https://assets.capsem.dev/profile.json",
                  "profile_hash": "{profile_hash}",
                  "profile_signature_url": "https://assets.capsem.dev/profile.json.minisig"
                }}
              }}
            }}
          }}
        }}"#
    ))
    .unwrap();
    let revision = manifest.revision("everyday-work", "2026.0520.1").unwrap();
    let verified =
        crate::profile_manifest::verify_installable_profile_payload(revision, payload).unwrap();
    let installed = install_verified_profile_payload(&roots, &verified).unwrap();
    fs::write(
        &installed.payload_path,
        br#"{"id":"everyday-work","tampered":true}"#,
    )
    .unwrap();

    let error =
        load_complete_installed_profile_revision(&roots, EVERYDAY_WORK_PROFILE_ID).unwrap_err();
    assert!(error.to_string().contains("payload hash"));
}

#[test]
fn install_verified_profile_payload_requires_corp_profile_root() {
    let temp = tempfile::tempdir().unwrap();
    let roots = test_roots(temp.path().join("base"), temp.path().join("user"));
    let payload = include_str!("../../../../schemas/fixtures/profile-v2-valid.json");
    let profile_hash = format!("blake3:{}", blake3::hash(payload.as_bytes()).to_hex());
    let manifest = crate::profile_manifest::ProfileManifest::from_json(&format!(
        r#"{{
          "format": 1,
          "profiles": {{
            "everyday-work": {{
              "current_revision": "2026.0520.1",
              "revisions": {{
                "2026.0520.1": {{
                  "status": "active",
                  "min_binary": "1.0.0",
                  "profile_url": "https://assets.capsem.dev/profile.json",
                  "profile_hash": "{profile_hash}",
                  "profile_signature_url": "https://assets.capsem.dev/profile.json.minisig"
                }}
              }}
            }}
          }}
        }}"#
    ))
    .unwrap();
    let verified = crate::profile_manifest::verify_installable_profile_payload(
        manifest.revision("everyday-work", "2026.0520.1").unwrap(),
        payload,
    )
    .unwrap();

    let error = install_verified_profile_payload(&roots, &verified).unwrap_err();

    assert!(matches!(error, SettingsProfilesError::Forbidden { .. }));
    assert!(format!("{error}").contains("no corp profile directory"));
}

#[tokio::test]
async fn reconcile_profile_revision_from_manifest_installs_active_revision() {
    let temp = tempfile::tempdir().unwrap();
    let base_dir = temp.path().join("base");
    let corp_dir = temp.path().join("corp");
    let user_dir = temp.path().join("user");
    fs::create_dir_all(&base_dir).unwrap();
    let mut roots = test_roots(base_dir, user_dir);
    roots.corp_dirs = vec![corp_dir.clone()];
    let payload_path = temp.path().join("profile.json");
    let signature_path = temp.path().join("profile.json.minisig");
    let payload = include_str!("../../../../schemas/fixtures/profile-v2-valid.json");
    let signature = include_str!("../../../../schemas/fixtures/profile-v2-valid.json.minisig");
    let pubkey = include_str!("../../../../schemas/fixtures/profile-v2-test.pub");
    fs::write(&payload_path, payload).unwrap();
    fs::write(&signature_path, signature).unwrap();
    let profile_hash = format!("blake3:{}", blake3::hash(payload.as_bytes()).to_hex());
    let manifest = crate::profile_manifest::ProfileManifest::from_json(&format!(
        r#"{{
          "format": 1,
          "profiles": {{
            "everyday-work": {{
              "current_revision": "2026.0520.1",
              "revisions": {{
                "2026.0520.1": {{
                  "status": "active",
                  "min_binary": "1.0.0",
                  "profile_url": "file://{}",
                  "profile_hash": "{profile_hash}",
                  "profile_signature_url": "file://{}"
                }}
              }}
            }}
          }}
        }}"#,
        payload_path.display(),
        signature_path.display(),
    ))
    .unwrap();

    let outcome = reconcile_profile_revision_from_manifest(
        &roots,
        manifest.revision("everyday-work", "2026.0520.1").unwrap(),
        pubkey,
    )
    .await
    .unwrap();

    let ProfileRevisionReconcileOutcome::Installed(installed) = outcome else {
        panic!("expected active revision install");
    };
    assert_eq!(installed.profile_id, EVERYDAY_WORK_PROFILE_ID);
    assert_eq!(installed.revision, "2026.0520.1");
    assert_eq!(installed.payload_hash, profile_hash);
    assert!(corp_dir.join("everyday-work.toml").exists());
    assert!(corp_dir
        .join(".catalog")
        .join("profiles")
        .join("everyday-work")
        .join("2026.0520.1")
        .join("profile.json")
        .exists());
    assert!(corp_dir
        .join(".catalog")
        .join("profiles")
        .join("everyday-work")
        .join("current.json")
        .exists());
}

#[tokio::test]
async fn reconcile_profile_revision_from_manifest_reinstalls_incomplete_active_revision() {
    let temp = tempfile::tempdir().unwrap();
    let base_dir = temp.path().join("base");
    let corp_dir = temp.path().join("corp");
    let user_dir = temp.path().join("user");
    fs::create_dir_all(&base_dir).unwrap();
    let mut roots = test_roots(base_dir, user_dir);
    roots.corp_dirs = vec![corp_dir.clone()];
    let payload_path = temp.path().join("profile.json");
    let signature_path = temp.path().join("profile.json.minisig");
    let payload = include_str!("../../../../schemas/fixtures/profile-v2-valid.json");
    let signature = include_str!("../../../../schemas/fixtures/profile-v2-valid.json.minisig");
    let pubkey = include_str!("../../../../schemas/fixtures/profile-v2-test.pub");
    fs::write(&payload_path, payload).unwrap();
    fs::write(&signature_path, signature).unwrap();
    let profile_hash = format!("blake3:{}", blake3::hash(payload.as_bytes()).to_hex());
    let record_dir = corp_dir
        .join(".catalog")
        .join("profiles")
        .join("everyday-work");
    fs::create_dir_all(&record_dir).unwrap();
    fs::write(
        record_dir.join("current.json"),
        format!(
            r#"{{
              "profile_id": "everyday-work",
              "revision": "2026.0520.1",
              "payload_hash": "{profile_hash}"
            }}"#
        ),
    )
    .unwrap();
    let manifest = crate::profile_manifest::ProfileManifest::from_json(&format!(
        r#"{{
          "format": 1,
          "profiles": {{
            "everyday-work": {{
              "current_revision": "2026.0520.1",
              "revisions": {{
                "2026.0520.1": {{
                  "status": "active",
                  "min_binary": "1.0.0",
                  "profile_url": "file://{}",
                  "profile_hash": "{profile_hash}",
                  "profile_signature_url": "file://{}"
                }}
              }}
            }}
          }}
        }}"#,
        payload_path.display(),
        signature_path.display(),
    ))
    .unwrap();

    let outcome = reconcile_profile_revision_from_manifest(
        &roots,
        manifest.revision("everyday-work", "2026.0520.1").unwrap(),
        pubkey,
    )
    .await
    .unwrap();

    assert!(matches!(
        outcome,
        ProfileRevisionReconcileOutcome::Installed(_)
    ));
    assert!(corp_dir.join("everyday-work.toml").exists());
    assert!(record_dir.join("2026.0520.1").join("profile.json").exists());
}

#[tokio::test]
async fn reconcile_profile_revision_from_manifest_skips_complete_active_revision() {
    let temp = tempfile::tempdir().unwrap();
    let base_dir = temp.path().join("base");
    let corp_dir = temp.path().join("corp");
    let user_dir = temp.path().join("user");
    fs::create_dir_all(&base_dir).unwrap();
    fs::create_dir_all(&corp_dir).unwrap();
    let mut roots = test_roots(base_dir, user_dir);
    roots.corp_dirs = vec![corp_dir.clone()];
    fs::write(
        corp_dir.join("everyday-work.toml"),
        toml::to_string_pretty(&Profile::everyday_work()).unwrap(),
    )
    .unwrap();
    let payload = include_str!("../../../../schemas/fixtures/profile-v2-valid.json");
    let profile_hash = format!("blake3:{}", blake3::hash(payload.as_bytes()).to_hex());
    let record_dir = corp_dir
        .join(".catalog")
        .join("profiles")
        .join("everyday-work")
        .join("2026.0520.1");
    fs::create_dir_all(&record_dir).unwrap();
    fs::write(record_dir.join("profile.json"), payload).unwrap();
    fs::write(
        record_dir.parent().unwrap().join("current.json"),
        format!(
            r#"{{
              "profile_id": "everyday-work",
              "revision": "2026.0520.1",
              "payload_hash": "{profile_hash}"
            }}"#
        ),
    )
    .unwrap();
    let manifest = crate::profile_manifest::ProfileManifest::from_json(&format!(
        r#"{{
          "format": 1,
          "profiles": {{
            "everyday-work": {{
              "current_revision": "2026.0520.1",
              "revisions": {{
                "2026.0520.1": {{
                  "status": "active",
                  "min_binary": "1.0.0",
                  "profile_url": "file:///definitely/not/read/profile.json",
                  "profile_hash": "{profile_hash}",
                  "profile_signature_url": "file:///definitely/not/read/profile.json.minisig"
                }}
              }}
            }}
          }}
        }}"#
    ))
    .unwrap();

    let outcome = reconcile_profile_revision_from_manifest(
        &roots,
        manifest.revision("everyday-work", "2026.0520.1").unwrap(),
        "unused",
    )
    .await
    .unwrap();

    let ProfileRevisionReconcileOutcome::Unchanged(record) = outcome else {
        panic!("expected complete active revision to be unchanged");
    };
    assert_eq!(record.revision, "2026.0520.1");
    assert_eq!(record.payload_hash, profile_hash);
}

#[tokio::test]
async fn reconcile_profile_revision_from_manifest_keeps_installed_deprecated_revision() {
    let temp = tempfile::tempdir().unwrap();
    let base_dir = temp.path().join("base");
    let corp_dir = temp.path().join("corp");
    let user_dir = temp.path().join("user");
    fs::create_dir_all(&base_dir).unwrap();
    fs::create_dir_all(&corp_dir).unwrap();
    let mut roots = test_roots(base_dir, user_dir);
    roots.corp_dirs = vec![corp_dir.clone()];
    fs::write(
        corp_dir.join("everyday-work.toml"),
        toml::to_string_pretty(&Profile::everyday_work()).unwrap(),
    )
    .unwrap();
    let record_dir = corp_dir
        .join(".catalog")
        .join("profiles")
        .join("everyday-work");
    fs::create_dir_all(&record_dir).unwrap();
    fs::write(
        record_dir.join("current.json"),
        r#"{
          "profile_id": "everyday-work",
          "revision": "2026.0520.1",
          "payload_hash": "blake3:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"
        }"#,
    )
    .unwrap();
    let manifest = crate::profile_manifest::ProfileManifest::from_json(
        r#"{
          "format": 1,
          "profiles": {
            "everyday-work": {
              "current_revision": "2026.0520.2",
              "revisions": {
                "2026.0520.1": {
                  "status": "deprecated",
                  "min_binary": "1.0.0",
                  "profile_url": "file:///definitely/not/read/profile.json",
                  "profile_hash": "blake3:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
                  "profile_signature_url": "file:///definitely/not/read/profile.json.minisig"
                },
                "2026.0520.2": {
                  "status": "active",
                  "min_binary": "1.0.0",
                  "profile_url": "https://assets.capsem.dev/profile.json",
                  "profile_hash": "blake3:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
                  "profile_signature_url": "https://assets.capsem.dev/profile.json.minisig"
                }
              }
            }
          }
        }"#,
    )
    .unwrap();

    let outcome = reconcile_profile_revision_from_manifest(
        &roots,
        manifest.revision("everyday-work", "2026.0520.1").unwrap(),
        "unused",
    )
    .await
    .unwrap();

    let ProfileRevisionReconcileOutcome::DeprecatedKept(record) = outcome else {
        panic!("expected deprecated installed revision to be kept");
    };
    assert_eq!(record.revision, "2026.0520.1");
    assert!(corp_dir.join("everyday-work.toml").exists());
    assert!(record_dir.join("current.json").exists());
}

#[tokio::test]
async fn reconcile_profile_revision_from_manifest_removes_revoked_current_revision() {
    let temp = tempfile::tempdir().unwrap();
    let base_dir = temp.path().join("base");
    let corp_dir = temp.path().join("corp");
    let user_dir = temp.path().join("user");
    fs::create_dir_all(&base_dir).unwrap();
    fs::create_dir_all(&corp_dir).unwrap();
    let mut roots = test_roots(base_dir, user_dir);
    roots.corp_dirs = vec![corp_dir.clone()];
    fs::write(
        corp_dir.join("everyday-work.toml"),
        toml::to_string_pretty(&Profile::everyday_work()).unwrap(),
    )
    .unwrap();
    let record_dir = corp_dir
        .join(".catalog")
        .join("profiles")
        .join("everyday-work");
    fs::create_dir_all(&record_dir).unwrap();
    fs::write(
        record_dir.join("current.json"),
        r#"{
          "profile_id": "everyday-work",
          "revision": "2026.0520.1",
          "payload_hash": "blake3:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"
        }"#,
    )
    .unwrap();
    let manifest = crate::profile_manifest::ProfileManifest::from_json(
        r#"{
          "format": 1,
          "profiles": {
            "everyday-work": {
              "current_revision": "2026.0520.2",
              "revisions": {
                "2026.0520.1": {
                  "status": "revoked",
                  "min_binary": "1.0.0",
                  "profile_url": "file:///definitely/not/read/profile.json",
                  "profile_hash": "blake3:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
                  "profile_signature_url": "file:///definitely/not/read/profile.json.minisig"
                },
                "2026.0520.2": {
                  "status": "active",
                  "min_binary": "1.0.0",
                  "profile_url": "https://assets.capsem.dev/profile.json",
                  "profile_hash": "blake3:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
                  "profile_signature_url": "https://assets.capsem.dev/profile.json.minisig"
                }
              }
            }
          }
        }"#,
    )
    .unwrap();

    let outcome = reconcile_profile_revision_from_manifest(
        &roots,
        manifest.revision("everyday-work", "2026.0520.1").unwrap(),
        "unused",
    )
    .await
    .unwrap();

    let ProfileRevisionReconcileOutcome::RevokedRemoved {
        profile_id,
        revision,
    } = outcome
    else {
        panic!("expected revoked current revision removal");
    };
    assert_eq!(profile_id, EVERYDAY_WORK_PROFILE_ID);
    assert_eq!(revision, "2026.0520.1");
    assert!(!corp_dir.join("everyday-work.toml").exists());
    assert!(!record_dir.join("current.json").exists());
}

#[test]
fn reconcile_absent_installed_profiles_removes_launchable_profile() {
    let temp = tempfile::tempdir().unwrap();
    let base_dir = temp.path().join("base");
    let corp_dir = temp.path().join("corp");
    let user_dir = temp.path().join("user");
    fs::create_dir_all(&base_dir).unwrap();
    fs::create_dir_all(&corp_dir).unwrap();
    let mut roots = test_roots(base_dir, user_dir);
    roots.corp_dirs = vec![corp_dir.clone()];
    fs::write(
        corp_dir.join("everyday-work.toml"),
        toml::to_string_pretty(&Profile::everyday_work()).unwrap(),
    )
    .unwrap();
    let record_dir = corp_dir
        .join(".catalog")
        .join("profiles")
        .join("everyday-work");
    fs::create_dir_all(record_dir.join("2026.0520.1")).unwrap();
    fs::write(record_dir.join("2026.0520.1").join("profile.json"), "{}").unwrap();
    fs::write(
        record_dir.join("current.json"),
        r#"{
          "profile_id": "everyday-work",
          "revision": "2026.0520.1",
          "payload_hash": "blake3:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"
        }"#,
    )
    .unwrap();
    let manifest = crate::profile_manifest::ProfileManifest::from_json(
        r#"{
          "format": 1,
          "profiles": {
            "coding": {
              "current_revision": "2026.0520.1",
              "revisions": {
                "2026.0520.1": {
                  "status": "active",
                  "min_binary": "1.0.0",
                  "profile_url": "https://assets.capsem.dev/coding/profile.json",
                  "profile_hash": "blake3:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
                  "profile_signature_url": "https://assets.capsem.dev/coding/profile.json.minisig"
                }
              }
            }
          }
        }"#,
    )
    .unwrap();

    let outcomes = reconcile_absent_installed_profiles_from_manifest(&roots, &manifest).unwrap();

    assert_eq!(
        outcomes,
        vec![ProfileRevisionReconcileOutcome::AbsentRemoved {
            profile_id: EVERYDAY_WORK_PROFILE_ID.to_string(),
            revision: "2026.0520.1".to_string()
        }]
    );
    assert!(!corp_dir.join("everyday-work.toml").exists());
    assert!(!record_dir.join("current.json").exists());
    assert!(record_dir.join("2026.0520.1").join("profile.json").exists());
}

#[test]
fn remove_installed_profile_revision_removes_launchable_state_only_for_selected_revision() {
    let temp = tempfile::tempdir().unwrap();
    let corp_dir = temp.path().join("corp");
    let mut roots = test_roots(temp.path().join("base"), temp.path().join("user"));
    roots.corp_dirs = vec![corp_dir.clone()];
    let record_dir = corp_dir
        .join(".catalog")
        .join("profiles")
        .join("everyday-work");
    fs::create_dir_all(record_dir.join("2026.0520.2")).unwrap();
    fs::write(
        corp_dir.join("everyday-work.toml"),
        "id = \"everyday-work\"\n",
    )
    .unwrap();
    fs::write(record_dir.join("2026.0520.2/profile.json"), "{}").unwrap();
    fs::write(
        record_dir.join("current.json"),
        r#"{
          "profile_id": "everyday-work",
          "revision": "2026.0520.2",
          "payload_hash": "blake3:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
        }"#,
    )
    .unwrap();

    let skipped =
        remove_installed_profile_revision(&roots, "everyday-work", Some("2026.0520.1")).unwrap();
    assert!(skipped.is_none());
    assert!(corp_dir.join("everyday-work.toml").exists());
    assert!(record_dir.join("current.json").exists());

    let removed = remove_installed_profile_revision(&roots, "everyday-work", Some("2026.0520.2"))
        .unwrap()
        .expect("selected installed revision should be removed");
    assert_eq!(removed.revision, "2026.0520.2");
    assert!(!corp_dir.join("everyday-work.toml").exists());
    assert!(!record_dir.join("current.json").exists());
    assert!(record_dir.join("2026.0520.2/profile.json").exists());
}

#[test]
fn installed_profile_asset_filenames_reads_current_payload_assets() {
    let temp = tempfile::tempdir().unwrap();
    let base_dir = temp.path().join("base");
    let corp_dir = temp.path().join("corp");
    let user_dir = temp.path().join("user");
    fs::create_dir_all(&base_dir).unwrap();
    let mut roots = test_roots(base_dir, user_dir);
    roots.corp_dirs = vec![corp_dir.clone()];
    let record_dir = corp_dir
        .join(".catalog")
        .join("profiles")
        .join("everyday-work");
    fs::create_dir_all(record_dir.join("2026.0520.1")).unwrap();
    fs::write(
        record_dir.join("2026.0520.1").join("profile.json"),
        include_str!("../../../../schemas/fixtures/profile-v2-valid.json"),
    )
    .unwrap();
    fs::write(
        record_dir.join("current.json"),
        r#"{
          "profile_id": "everyday-work",
          "revision": "2026.0520.1",
          "payload_hash": "blake3:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"
        }"#,
    )
    .unwrap();

    let filenames = installed_profile_asset_filenames(&roots).unwrap();

    assert!(filenames.contains("vmlinuz-aaaaaaaaaaaaaaaa"));
    assert!(filenames.contains("initrd-bbbbbbbbbbbbbbbb.img"));
    assert!(filenames.contains("rootfs-cccccccccccccccc.squashfs"));
}

#[test]
fn installed_profile_asset_filenames_ignores_archived_payload_without_current_record() {
    let temp = tempfile::tempdir().unwrap();
    let base_dir = temp.path().join("base");
    let corp_dir = temp.path().join("corp");
    let user_dir = temp.path().join("user");
    fs::create_dir_all(&base_dir).unwrap();
    let mut roots = test_roots(base_dir, user_dir);
    roots.corp_dirs = vec![corp_dir.clone()];
    let archived = corp_dir
        .join(".catalog")
        .join("profiles")
        .join("everyday-work")
        .join("2026.0520.1");
    fs::create_dir_all(&archived).unwrap();
    fs::write(
        archived.join("profile.json"),
        include_str!("../../../../schemas/fixtures/profile-v2-valid.json"),
    )
    .unwrap();

    let filenames = installed_profile_asset_filenames(&roots).unwrap();

    assert!(filenames.is_empty());
}

#[test]
fn user_profile_fork_from_builtin_profile() {
    let temp = tempfile::tempdir().unwrap();
    let base_dir = temp.path().join("base");
    let user_dir = temp.path().join("user");
    fs::create_dir_all(&base_dir).unwrap();
    let roots = test_roots(base_dir, user_dir);

    let forked = fork_user_profile(
        &roots,
        EVERYDAY_WORK_PROFILE_ID,
        "daily-strict",
        "Daily Strict",
    )
    .unwrap();

    assert_eq!(forked.profile.id, "daily-strict");
    assert_eq!(forked.profile.name, "Daily Strict");
    assert_eq!(forked.source, ProfileSource::User);
    assert!(discover_profiles(&roots)
        .unwrap()
        .get("daily-strict")
        .is_some());
}

#[test]
fn user_profile_create_respects_governance() {
    let temp = tempfile::tempdir().unwrap();
    let base_dir = temp.path().join("base");
    let user_dir = temp.path().join("user");
    fs::create_dir_all(&base_dir).unwrap();
    let mut roots = test_roots(base_dir, user_dir);
    roots.allow_user_profiles = false;

    let error = create_user_profile(
        &roots,
        profile_value("custom", "Custom", ProfileType::Coding),
    )
    .unwrap_err();

    assert!(matches!(error, SettingsProfilesError::Forbidden { .. }));
}

#[test]
fn user_profile_fork_respects_governance() {
    let temp = tempfile::tempdir().unwrap();
    let base_dir = temp.path().join("base");
    let user_dir = temp.path().join("user");
    fs::create_dir_all(&base_dir).unwrap();
    let mut roots = test_roots(base_dir, user_dir);
    roots.allow_user_fork = false;

    let error =
        fork_user_profile(&roots, EVERYDAY_WORK_PROFILE_ID, "forked", "Forked").unwrap_err();

    assert!(matches!(error, SettingsProfilesError::Forbidden { .. }));
}

#[test]
fn user_profile_delete_respects_governance() {
    let temp = tempfile::tempdir().unwrap();
    let base_dir = temp.path().join("base");
    let user_dir = temp.path().join("user");
    fs::create_dir_all(&base_dir).unwrap();
    let mut roots = test_roots(base_dir, user_dir);
    create_user_profile(
        &roots,
        profile_value("custom", "Custom", ProfileType::Coding),
    )
    .unwrap();
    roots.allow_user_delete = false;

    let error = delete_user_profile(&roots, "custom").unwrap_err();

    assert!(matches!(error, SettingsProfilesError::Forbidden { .. }));
}

#[test]
fn user_profile_create_rejects_duplicate_user_file() {
    let temp = tempfile::tempdir().unwrap();
    let base_dir = temp.path().join("base");
    let user_dir = temp.path().join("user");
    fs::create_dir_all(&base_dir).unwrap();
    let roots = test_roots(base_dir, user_dir);
    create_user_profile(
        &roots,
        profile_value("custom", "Custom", ProfileType::Coding),
    )
    .unwrap();

    let error = create_user_profile(
        &roots,
        profile_value("custom", "Custom Again", ProfileType::Coding),
    )
    .unwrap_err();

    assert!(matches!(
        error,
        SettingsProfilesError::DuplicateProfile { .. }
    ));
}

#[test]
fn user_profile_update_missing_profile_errors_clearly() {
    let temp = tempfile::tempdir().unwrap();
    let base_dir = temp.path().join("base");
    let user_dir = temp.path().join("user");
    fs::create_dir_all(&base_dir).unwrap();
    let roots = test_roots(base_dir, user_dir);

    let error = update_user_profile(
        &roots,
        profile_value("missing", "Missing", ProfileType::Coding),
    )
    .unwrap_err();

    assert!(matches!(
        error,
        SettingsProfilesError::ProfileNotFound { .. }
    ));
}

#[test]
fn resolve_effective_vm_settings_uses_default_profile_with_provenance() {
    let temp = tempfile::tempdir().unwrap();
    let base_dir = temp.path().join("base");
    let user_dir = temp.path().join("user");
    fs::create_dir_all(&base_dir).unwrap();
    let roots = test_roots(base_dir, user_dir);

    let effective = resolve_effective_vm_settings(&roots, None).unwrap();

    assert_eq!(effective.profile_id, EVERYDAY_WORK_PROFILE_ID);
    assert_eq!(effective.profile.provenance.source, ProfileSource::BuiltIn);
    assert_eq!(effective.vm.provenance.toml_path, "vm");
    // Slice 6b.5: catch-all rules now use the canonical per-type
    // ids (dns.default, http.default_read, http.default_write,
    // model.default, mcp.default) at priority 1000.
    let dns_catch_all = effective
        .rules
        .iter()
        .find(|rule| rule.id == "dns.default")
        .expect("dns catch-all expected");
    assert!(dns_catch_all.derived);
    assert_eq!(dns_catch_all.priority, RULE_CATCH_ALL_PRIORITY);
    assert_eq!(
        dns_catch_all.provenance.toml_path,
        "security.capabilities.network_egress"
    );
    assert!(
        dns_catch_all.provenance.locked,
        "derived catch-all rules from locked profiles must carry locked provenance"
    );

    // Every runtime callback gets exactly one catch-all.
    let expected_ids = [
        "dns.default",
        "http.default_read",
        "http.default_write",
        "model.default",
        "mcp.default",
    ];
    for id in expected_ids {
        assert!(
            effective.rules.iter().any(|rule| rule.id == id),
            "missing catch-all '{id}'"
        );
    }
}

#[test]
fn resolve_effective_vm_settings_includes_profile_and_derived_rules() {
    let temp = tempfile::tempdir().unwrap();
    let base_dir = temp.path().join("base");
    let user_dir = temp.path().join("user");
    fs::create_dir_all(&base_dir).unwrap();
    let roots = test_roots(base_dir, user_dir);
    let mut profile = profile_value("strict", "Strict", ProfileType::Coding);
    profile.security.capabilities.network_egress = CapabilityMode::Block;
    profile.security.rules.mcp.insert(
        "ask-shell-tool".to_string(),
        ProfileRule {
            callback: "mcp.request".to_string(),
            condition: "tool.name == 'shell'".to_string(),
            decision: RuleDecision::Ask,
            priority: 500,
            rewrite_target: None,
            rewrite_value: None,
            strip_request_headers: Vec::new(),
            strip_response_headers: Vec::new(),
            reason: Some("Ask before shell tool use.".to_string()),
        },
    );
    create_user_profile(&roots, profile).unwrap();

    let effective = resolve_effective_vm_settings(&roots, Some("strict")).unwrap();
    // network_egress = Block drives dns/http/model catch-alls
    // to Block at priority 1000.
    let dns_catch_all = effective
        .rules
        .iter()
        .find(|rule| rule.id == "dns.default")
        .unwrap();
    assert_eq!(dns_catch_all.decision, RuleDecision::Block);
    assert!(dns_catch_all.derived);
    assert_eq!(dns_catch_all.priority, RULE_CATCH_ALL_PRIORITY);
    assert_eq!(dns_catch_all.provenance.source, ProfileSource::User);
    for id in ["http.default_read", "http.default_write", "model.default"] {
        let rule = effective
            .rules
            .iter()
            .find(|rule| rule.id == id)
            .unwrap_or_else(|| panic!("missing '{id}'"));
        assert_eq!(
            rule.decision,
            RuleDecision::Block,
            "{id} should follow network_egress = Block"
        );
    }

    let profile_rule = effective
        .rules
        .iter()
        .find(|rule| rule.id == "mcp.ask-shell-tool")
        .unwrap();
    assert!(!profile_rule.derived);
    assert_eq!(
        profile_rule.provenance.toml_path,
        "security.rules.mcp.ask-shell-tool"
    );
}

#[test]
fn resolve_effective_vm_settings_errors_for_missing_profile() {
    let temp = tempfile::tempdir().unwrap();
    let base_dir = temp.path().join("base");
    let user_dir = temp.path().join("user");
    fs::create_dir_all(&base_dir).unwrap();
    let roots = test_roots(base_dir, user_dir);

    let error = resolve_effective_vm_settings(&roots, Some("missing")).unwrap_err();

    assert!(matches!(
        error,
        SettingsProfilesError::ProfileNotFound { .. }
    ));
}

#[test]
fn vm_effective_settings_round_trip_attaches_to_session_dir() {
    let temp = tempfile::tempdir().unwrap();
    let base_dir = temp.path().join("base");
    let user_dir = temp.path().join("user");
    let session_dir = temp.path().join("sessions").join("vm-1");
    fs::create_dir_all(&base_dir).unwrap();
    let roots = test_roots(base_dir, user_dir);
    let effective = resolve_effective_vm_settings(&roots, None).unwrap();

    write_vm_effective_settings(&session_dir, &effective).unwrap();
    let loaded = load_vm_effective_settings(&session_dir).unwrap();

    assert_eq!(
        vm_effective_settings_path(&session_dir),
        session_dir.join("vm-effective-settings.toml")
    );
    assert_eq!(loaded.profile_id, EVERYDAY_WORK_PROFILE_ID);
    assert_eq!(loaded.rules.len(), effective.rules.len());
    assert_eq!(loaded, effective);
}

#[test]
fn vm_effective_settings_missing_file_errors_clearly() {
    let temp = tempfile::tempdir().unwrap();

    let error = load_vm_effective_settings(temp.path()).unwrap_err();

    assert!(matches!(error, SettingsProfilesError::ReadFile { .. }));
}

#[test]
fn vm_effective_settings_corrupt_file_errors_clearly() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(
        vm_effective_settings_path(temp.path()),
        r#"
profile_id = "broken"
rules = "not a rule list"
"#,
    )
    .unwrap();

    let error = load_vm_effective_settings(temp.path()).unwrap_err();

    assert!(matches!(error, SettingsProfilesError::Parse { .. }));
}

#[test]
fn profile_descriptors_cover_security_and_ui_builder_inputs() {
    let service_paths = service_setting_descriptors()
        .into_iter()
        .map(|descriptor| descriptor.path)
        .collect::<Vec<_>>();
    assert!(service_paths.contains(&"assets.image_roots"));
    assert!(service_paths.contains(&"assets.download_base_url"));
    assert!(service_paths.contains(&"telemetry.endpoint"));
    assert!(service_paths.contains(&"remote_policy.endpoint"));

    let profile_paths = profile_setting_descriptors()
        .into_iter()
        .map(|descriptor| descriptor.path)
        .collect::<Vec<_>>();
    assert!(profile_paths.contains(&"extends_profile_id"));
    assert!(profile_paths.contains(&"packages"));
    assert!(profile_paths.contains(&"tools"));
    assert!(profile_paths.contains(&"vm.assets"));
    assert!(profile_paths.contains(&"security.capabilities"));
    assert!(profile_paths.contains(&"security.rules"));
}

fn test_roots(base_dir: PathBuf, user_dir: PathBuf) -> ProfileRootSettings {
    ProfileRootSettings {
        base_dirs: vec![base_dir],
        corp_dirs: Vec::new(),
        user_dirs: vec![user_dir],
        default_profile: EVERYDAY_WORK_PROFILE_ID.to_string(),
        allow_user_profiles: true,
        allow_user_fork: true,
        allow_user_delete: true,
    }
}

fn profile_value(id: &str, name: &str, profile_type: ProfileType) -> Profile {
    let mut profile = Profile::everyday_work();
    profile.id = id.to_string();
    profile.name = name.to_string();
    profile.best_for = format!("{name} sessions.");
    profile.profile_type = profile_type;
    profile
}

fn profile_toml(id: &str, name: &str, profile_type: &str) -> String {
    format!(
        r#"
version = 1
id = "{id}"
name = "{name}"
best_for = "{name} sessions."
profile_type = "{profile_type}"
"#
    )
}

fn profile_toml_with_parent(id: &str, name: &str, profile_type: &str, parent: &str) -> String {
    format!(
        r#"
version = 1
id = "{id}"
name = "{name}"
best_for = "{name} sessions."
profile_type = "{profile_type}"
extends_profile_id = "{parent}"
"#
    )
}

/// Build a catalog directly from in-memory `Profile` values,
/// bypassing on-disk discovery. Used by parent-chain validation
/// tests that need to inject cycles or unknown parents -- shapes
/// that `Profile::from_toml_str` rejects up front.
fn catalog_from_profiles(profiles: Vec<Profile>) -> ProfileCatalog {
    let mut catalog = ProfileCatalog::default();
    for profile in profiles {
        let record = ProfileRecord {
            profile,
            source: ProfileSource::Base,
            path: None,
            locked: false,
        };
        catalog.profiles.insert(record.profile.id.clone(), record);
    }
    catalog
}

fn parented_profile(id: &str, parent: Option<&str>) -> Profile {
    let mut profile = profile_value(id, id, ProfileType::Coding);
    profile.extends_profile_id = parent.map(str::to_string);
    profile
}

#[test]
fn validate_parent_chain_accepts_single_level_inheritance() {
    let catalog = catalog_from_profiles(vec![
        parented_profile("root", None),
        parented_profile("child", Some("root")),
    ]);
    validate_parent_chain(&catalog).unwrap();
}

#[test]
fn validate_parent_chain_accepts_max_depth_chain() {
    // Eight ancestors + one leaf = exactly MAX_PROFILE_INHERITANCE_DEPTH edges.
    let mut profiles = Vec::new();
    profiles.push(parented_profile("p0", None));
    for i in 1..=MAX_PROFILE_INHERITANCE_DEPTH {
        let id = format!("p{i}");
        let parent = format!("p{}", i - 1);
        profiles.push(parented_profile(&id, Some(&parent)));
    }
    let catalog = catalog_from_profiles(profiles);
    validate_parent_chain(&catalog).unwrap();
}

#[test]
fn validate_parent_chain_rejects_depth_overflow() {
    let mut profiles = Vec::new();
    profiles.push(parented_profile("p0", None));
    for i in 1..=MAX_PROFILE_INHERITANCE_DEPTH + 1 {
        let id = format!("p{i}");
        let parent = format!("p{}", i - 1);
        profiles.push(parented_profile(&id, Some(&parent)));
    }
    let catalog = catalog_from_profiles(profiles);
    let error = validate_parent_chain(&catalog).unwrap_err();
    assert!(
        matches!(
            error,
            SettingsProfilesError::InheritanceDepthExceeded { .. }
        ),
        "expected InheritanceDepthExceeded, got {error:?}"
    );
}

#[test]
fn validate_parent_chain_rejects_unknown_parent() {
    let catalog = catalog_from_profiles(vec![parented_profile("child", Some("ghost"))]);
    let error = validate_parent_chain(&catalog).unwrap_err();
    assert!(
        matches!(
            error,
            SettingsProfilesError::UnknownParentProfile { ref parent, .. }
                if parent == "ghost"
        ),
        "expected UnknownParentProfile(parent=ghost), got {error:?}"
    );
}

#[test]
fn validate_parent_chain_rejects_two_node_cycle() {
    // A -> B -> A. Profile::validate() rejects the self-loop case
    // (`A -> A`); the two-node form crosses records, so only the
    // catalog-level validator can catch it.
    let catalog = catalog_from_profiles(vec![
        parented_profile("a", Some("b")),
        parented_profile("b", Some("a")),
    ]);
    let error = validate_parent_chain(&catalog).unwrap_err();
    assert!(
        matches!(error, SettingsProfilesError::InheritanceCycle { .. }),
        "expected InheritanceCycle, got {error:?}"
    );
}

#[test]
fn validate_parent_chain_rejects_three_node_cycle() {
    let catalog = catalog_from_profiles(vec![
        parented_profile("a", Some("b")),
        parented_profile("b", Some("c")),
        parented_profile("c", Some("a")),
    ]);
    let error = validate_parent_chain(&catalog).unwrap_err();
    assert!(
        matches!(error, SettingsProfilesError::InheritanceCycle { .. }),
        "expected InheritanceCycle, got {error:?}"
    );
}

#[test]
fn resolve_ancestor_chain_returns_root_to_leaf_order() {
    let catalog = catalog_from_profiles(vec![
        parented_profile("root", None),
        parented_profile("mid", Some("root")),
        parented_profile("leaf", Some("mid")),
    ]);
    let chain = resolve_ancestor_chain(&catalog, "leaf").unwrap();
    let ids: Vec<&str> = chain.iter().map(|r| r.profile.id.as_str()).collect();
    assert_eq!(ids, vec!["root", "mid", "leaf"]);
}

#[test]
fn resolve_ancestor_chain_errors_for_missing_leaf() {
    let catalog = catalog_from_profiles(vec![parented_profile("root", None)]);
    let error = resolve_ancestor_chain(&catalog, "ghost").unwrap_err();
    assert!(
        matches!(error, SettingsProfilesError::ProfileNotFound { ref id } if id == "ghost"),
        "expected ProfileNotFound(ghost), got {error:?}"
    );
}

#[test]
fn discover_profiles_fails_closed_on_unknown_parent() {
    let temp = tempfile::tempdir().unwrap();
    let base_dir = temp.path().join("base");
    let user_dir = temp.path().join("user");
    fs::create_dir_all(&base_dir).unwrap();
    fs::write(
        base_dir.join("orphan.toml"),
        profile_toml_with_parent("orphan", "Orphan", "coding", "ghost"),
    )
    .unwrap();

    let roots = test_roots(base_dir, user_dir);
    let error = discover_profiles(&roots).unwrap_err();
    assert!(
        matches!(
            error,
            SettingsProfilesError::UnknownParentProfile { ref parent, .. }
                if parent == "ghost"
        ),
        "expected UnknownParentProfile(parent=ghost), got {error:?}"
    );
}

fn write_profile(dir: &Path, id: &str, body: &str) {
    fs::write(dir.join(format!("{id}.toml")), body).unwrap();
}

#[test]
fn layered_merge_child_rule_overrides_parent_rule_by_name() {
    let temp = tempfile::tempdir().unwrap();
    let base_dir = temp.path().join("base");
    let user_dir = temp.path().join("user");
    fs::create_dir_all(&base_dir).unwrap();

    write_profile(
        &base_dir,
        "parent",
        r#"
version = 1
id = "parent"
name = "Parent"
best_for = "Parent."
profile_type = "coding"

[security.rules.http.block-secret]
on = "http.request"
if = "request.data.contains_secret"
decision = "block"
"#,
    );
    write_profile(
        &base_dir,
        "child",
        r#"
version = 1
id = "child"
name = "Child"
best_for = "Child."
profile_type = "coding"
extends_profile_id = "parent"

[security.rules.http.block-secret]
on = "http.request"
if = "request.data.contains_secret"
decision = "allow"
reason = "child relaxes the parent block"
"#,
    );

    let roots = test_roots(base_dir, user_dir);
    let effective = resolve_effective_vm_settings(&roots, Some("child")).unwrap();

    let rule = effective
        .rules
        .iter()
        .find(|rule| rule.id == "http.block-secret")
        .expect("child rule should be present");
    assert_eq!(rule.decision, RuleDecision::Allow);
    assert_eq!(
        rule.reason.as_deref(),
        Some("child relaxes the parent block")
    );
    assert_eq!(rule.provenance.profile_id, "child");
}

#[test]
fn layered_merge_inherits_parent_rules_when_child_omits() {
    let temp = tempfile::tempdir().unwrap();
    let base_dir = temp.path().join("base");
    let user_dir = temp.path().join("user");
    fs::create_dir_all(&base_dir).unwrap();

    write_profile(
        &base_dir,
        "parent",
        r#"
version = 1
id = "parent"
name = "Parent"
best_for = "Parent."
profile_type = "coding"

[security.rules.http.parent-only]
on = "http.request"
if = "request.data.contains_secret"
decision = "block"
"#,
    );
    write_profile(
        &base_dir,
        "child",
        r#"
version = 1
id = "child"
name = "Child"
best_for = "Child."
profile_type = "coding"
extends_profile_id = "parent"

[security.rules.http.child-only]
on = "http.request"
if = "true"
decision = "allow"
"#,
    );

    let roots = test_roots(base_dir, user_dir);
    let effective = resolve_effective_vm_settings(&roots, Some("child")).unwrap();

    let parent_rule = effective
        .rules
        .iter()
        .find(|rule| rule.id == "http.parent-only")
        .expect("parent rule should be inherited");
    assert_eq!(parent_rule.provenance.profile_id, "parent");

    let child_rule = effective
        .rules
        .iter()
        .find(|rule| rule.id == "http.child-only")
        .expect("child rule should be present");
    assert_eq!(child_rule.provenance.profile_id, "child");
}

#[test]
fn layered_merge_records_inherited_from_on_sections() {
    let temp = tempfile::tempdir().unwrap();
    let base_dir = temp.path().join("base");
    let user_dir = temp.path().join("user");
    fs::create_dir_all(&base_dir).unwrap();

    write_profile(
        &base_dir,
        "root",
        r#"
version = 1
id = "root"
name = "Root"
best_for = "Root."
profile_type = "coding"
"#,
    );
    write_profile(
        &base_dir,
        "mid",
        r#"
version = 1
id = "mid"
name = "Mid"
best_for = "Mid."
profile_type = "coding"
extends_profile_id = "root"
"#,
    );
    write_profile(
        &base_dir,
        "leaf",
        r#"
version = 1
id = "leaf"
name = "Leaf"
best_for = "Leaf."
profile_type = "coding"
extends_profile_id = "mid"
"#,
    );

    let roots = test_roots(base_dir, user_dir);
    let effective = resolve_effective_vm_settings(&roots, Some("leaf")).unwrap();

    assert_eq!(effective.profile_id, "leaf");
    assert_eq!(effective.ai.inherited_from, vec!["root", "mid"]);
    assert_eq!(effective.security.inherited_from, vec!["root", "mid"]);
    assert_eq!(effective.skills.inherited_from, vec!["root", "mid"]);
    // The leaf's own provenance is still attributed to the leaf
    // -- inherited_from is the ancestor list, not the contributor.
    assert_eq!(effective.security.provenance.profile_id, "leaf");
}

#[test]
fn layered_merge_unions_mcp_connectors_with_child_override_per_key() {
    let temp = tempfile::tempdir().unwrap();
    let base_dir = temp.path().join("base");
    let user_dir = temp.path().join("user");
    fs::create_dir_all(&base_dir).unwrap();

    write_profile(
        &base_dir,
        "parent",
        r#"
version = 1
id = "parent"
name = "Parent"
best_for = "Parent."
profile_type = "coding"

[mcpServers.github]
enabled = true
command = "npx"
[mcpServers.github.capsem]
allowed_tools = ["repo.read"]

[mcpServers.shared]
enabled = true
command = "python"
[mcpServers.shared.capsem]
allowed_tools = ["parent.tool"]
"#,
    );
    write_profile(
        &base_dir,
        "child",
        r#"
version = 1
id = "child"
name = "Child"
best_for = "Child."
profile_type = "coding"
extends_profile_id = "parent"

[mcpServers.shared]
enabled = true
command = "node"
[mcpServers.shared.capsem]
allowed_tools = ["child.tool"]

[mcpServers.local]
enabled = true
command = "uvx"
[mcpServers.local.capsem]
allowed_tools = ["local.tool"]
"#,
    );

    let roots = test_roots(base_dir, user_dir);
    let effective = resolve_effective_vm_settings(&roots, Some("child")).unwrap();

    let connectors = &effective.mcp.value.connectors;
    // Parent-only key flows through.
    assert!(connectors.contains_key("github"));
    // Child-only key is added.
    assert!(connectors.contains_key("local"));
    // Shared key: child wins entirely (not partial merge).
    let shared = connectors.get("shared").expect("shared connector");
    assert_eq!(shared.capsem.allowed_tools, vec!["child.tool".to_string()]);
}

#[test]
fn layered_merge_unions_skills_lists_with_dedup() {
    let temp = tempfile::tempdir().unwrap();
    let base_dir = temp.path().join("base");
    let user_dir = temp.path().join("user");
    fs::create_dir_all(&base_dir).unwrap();

    write_profile(
        &base_dir,
        "parent",
        r#"
version = 1
id = "parent"
name = "Parent"
best_for = "Parent."
profile_type = "coding"

[skills]
groups = ["dev"]
enabled = ["dev-sprint", "shared-skill"]
"#,
    );
    write_profile(
        &base_dir,
        "child",
        r#"
version = 1
id = "child"
name = "Child"
best_for = "Child."
profile_type = "coding"
extends_profile_id = "parent"

[skills]
groups = ["dev", "ops"]
enabled = ["shared-skill", "child-skill"]
"#,
    );

    let roots = test_roots(base_dir, user_dir);
    let effective = resolve_effective_vm_settings(&roots, Some("child")).unwrap();

    let skills = &effective.skills.value;
    // Each id appears exactly once; child positions win.
    assert_eq!(skills.groups, vec!["dev".to_string(), "ops".to_string()]);
    assert_eq!(
        skills.enabled,
        vec![
            "dev-sprint".to_string(),
            "shared-skill".to_string(),
            "child-skill".to_string()
        ]
    );
}

#[test]
fn layered_merge_unions_package_tool_and_asset_contracts_by_key() {
    let temp = tempfile::tempdir().unwrap();
    let base_dir = temp.path().join("base");
    let user_dir = temp.path().join("user");
    fs::create_dir_all(&base_dir).unwrap();

    write_profile(
        &base_dir,
        "parent",
        r#"
version = 1
id = "parent"
name = "Parent"
best_for = "Parent."
profile_type = "coding"

[packages.runtimes]
python = "3.12.3"
node = "22.1.0"

[tools.capsem_doctor]
version = "2026.05.18"
required = true
source = "guest"

[vm.assets.arm64.kernel]
url = "https://assets.capsem.dev/parent/arm64/vmlinuz"
hash = "blake3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
signature_url = "https://assets.capsem.dev/parent/arm64/vmlinuz.minisig"
size = 10
content_type = "application/octet-stream"

[vm.assets.arm64.initrd]
url = "https://assets.capsem.dev/parent/arm64/initrd.img"
hash = "blake3:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
signature_url = "https://assets.capsem.dev/parent/arm64/initrd.img.minisig"
size = 11
content_type = "application/octet-stream"

[vm.assets.arm64.rootfs]
url = "https://assets.capsem.dev/parent/arm64/rootfs.squashfs"
hash = "blake3:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
signature_url = "https://assets.capsem.dev/parent/arm64/rootfs.squashfs.minisig"
size = 12
content_type = "application/vnd.squashfs"
"#,
    );
    write_profile(
        &base_dir,
        "child",
        r#"
version = 1
id = "child"
name = "Child"
best_for = "Child."
profile_type = "coding"
extends_profile_id = "parent"

[packages.runtimes]
python = "3.13.0"
uv = "0.4.30"

[tools.uv]
version = "0.4.30"
required = true
source = "guest"

[vm.assets.x86_64.kernel]
url = "https://assets.capsem.dev/child/x86_64/vmlinuz"
hash = "blake3:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
signature_url = "https://assets.capsem.dev/child/x86_64/vmlinuz.minisig"
size = 20
content_type = "application/octet-stream"

[vm.assets.x86_64.initrd]
url = "https://assets.capsem.dev/child/x86_64/initrd.img"
hash = "blake3:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"
signature_url = "https://assets.capsem.dev/child/x86_64/initrd.img.minisig"
size = 21
content_type = "application/octet-stream"

[vm.assets.x86_64.rootfs]
url = "https://assets.capsem.dev/child/x86_64/rootfs.squashfs"
hash = "blake3:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
signature_url = "https://assets.capsem.dev/child/x86_64/rootfs.squashfs.minisig"
size = 22
content_type = "application/vnd.squashfs"
"#,
    );

    let roots = test_roots(base_dir, user_dir);
    let effective = resolve_effective_vm_settings(&roots, Some("child")).unwrap();

    assert_eq!(effective.packages.value.runtimes["python"], "3.13.0");
    assert_eq!(effective.packages.value.runtimes["node"], "22.1.0");
    assert_eq!(effective.packages.value.runtimes["uv"], "0.4.30");
    assert!(effective.tools.value.contains_key("capsem_doctor"));
    assert!(effective.tools.value.contains_key("uv"));
    assert_eq!(effective.vm.value.assets["arm64"].rootfs.size, 12);
    assert_eq!(effective.vm.value.assets["x86_64"].rootfs.size, 22);
    assert_eq!(effective.packages.inherited_from, vec!["parent"]);
    assert_eq!(effective.tools.inherited_from, vec!["parent"]);
}

#[test]
fn layered_merge_capabilities_are_atomic_child_wins() {
    let temp = tempfile::tempdir().unwrap();
    let base_dir = temp.path().join("base");
    let user_dir = temp.path().join("user");
    fs::create_dir_all(&base_dir).unwrap();

    write_profile(
        &base_dir,
        "parent",
        r#"
version = 1
id = "parent"
name = "Parent"
best_for = "Parent."
profile_type = "coding"

[security.capabilities]
credential_brokerage = "block"
pii_detection = "block"
mcp_rag = "block"
mcp_tools = "block"
network_egress = "block"
file_boundaries = "block"
audit = "audit"
"#,
    );
    // Child sets only one capability explicitly. Because
    // capabilities are an atomic struct, the parent's other
    // `"block"` values are NOT silently inherited; the leaf's
    // schema-default `"ask"` wins.
    write_profile(
        &base_dir,
        "child",
        r#"
version = 1
id = "child"
name = "Child"
best_for = "Child."
profile_type = "coding"
extends_profile_id = "parent"

[security.capabilities]
credential_brokerage = "allow"
"#,
    );

    let roots = test_roots(base_dir, user_dir);
    let effective = resolve_effective_vm_settings(&roots, Some("child")).unwrap();

    let caps = &effective.security.value.capabilities;
    assert_eq!(caps.credential_brokerage, CapabilityMode::Allow);
    // Documented contract: child wins entirely, so parent's
    // `block` on these does NOT bleed through.
    assert_eq!(caps.pii_detection, CapabilityMode::Ask);
    assert_eq!(caps.mcp_rag, CapabilityMode::Ask);
}

#[test]
fn layered_merge_no_ancestor_chain_leaves_inherited_from_empty() {
    // Selecting the built-in everyday-work profile (no parent)
    // must still produce a coherent EffectiveVmSettings with
    // empty `inherited_from`. Regression guard against the new
    // chain code path silently appending the leaf to its own
    // ancestor list.
    let temp = tempfile::tempdir().unwrap();
    let base_dir = temp.path().join("base");
    let user_dir = temp.path().join("user");
    fs::create_dir_all(&base_dir).unwrap();

    let roots = test_roots(base_dir, user_dir);
    let effective = resolve_effective_vm_settings(&roots, None).unwrap();
    assert_eq!(effective.profile_id, EVERYDAY_WORK_PROFILE_ID);
    assert!(effective.ai.inherited_from.is_empty());
    assert!(effective.security.inherited_from.is_empty());
    assert!(effective.mcp.inherited_from.is_empty());
    assert!(effective.skills.inherited_from.is_empty());
    assert!(effective.vm.inherited_from.is_empty());
}

#[test]
fn resolve_effective_vm_settings_errors_on_cyclic_parent_chain() {
    // Build an on-disk catalog where two non-builtin profiles
    // reference each other. The cycle must surface through the
    // production resolve path (not just the validator helper),
    // proving the validator wiring blocks runtime resolve.
    let temp = tempfile::tempdir().unwrap();
    let base_dir = temp.path().join("base");
    let user_dir = temp.path().join("user");
    fs::create_dir_all(&base_dir).unwrap();
    fs::write(
        base_dir.join("alpha.toml"),
        profile_toml_with_parent("alpha", "Alpha", "coding", "beta"),
    )
    .unwrap();
    fs::write(
        base_dir.join("beta.toml"),
        profile_toml_with_parent("beta", "Beta", "coding", "alpha"),
    )
    .unwrap();

    let roots = test_roots(base_dir, user_dir);
    let error = resolve_effective_vm_settings(&roots, Some("alpha")).unwrap_err();
    assert!(
        matches!(error, SettingsProfilesError::InheritanceCycle { .. }),
        "expected InheritanceCycle, got {error:?}"
    );
}

#[test]
fn derived_capability_rules_carry_ownership_metadata() {
    // Slice 6b.1: capability-derived rules are uneditable and
    // point back at their owning setting so the UI can render
    // "managed by Security capability · network-egress" and
    // the future mutation gate (6b.8) can refuse direct edits.
    let temp = tempfile::tempdir().unwrap();
    let base_dir = temp.path().join("base");
    let user_dir = temp.path().join("user");
    fs::create_dir_all(&base_dir).unwrap();
    let roots = test_roots(base_dir, user_dir);

    let effective = resolve_effective_vm_settings(&roots, None).unwrap();
    let capability_rules: Vec<&EffectiveRule> =
        effective.rules.iter().filter(|rule| rule.derived).collect();
    assert!(!capability_rules.is_empty(), "capability rules expected");
    for rule in &capability_rules {
        assert!(
            !rule.editable,
            "capability-derived rule {id} must be uneditable",
            id = rule.id
        );
        let owner = rule
            .owner_setting_path
            .as_deref()
            .expect("derived rule must carry owner_setting_path");
        assert!(
            owner.starts_with("security.capabilities."),
            "owner_setting_path '{owner}' should point at the capability"
        );
        let label = rule
            .owner_setting_label
            .as_deref()
            .expect("derived rule must carry owner_setting_label");
        assert!(
            label.starts_with("Capability default"),
            "label '{label}' should identify the capability default"
        );
    }
}

#[test]
fn hand_authored_profile_rule_is_editable_with_no_owner_setting() {
    // Slice 6b.1: rules that live in a `security.rules.<type>.<name>`
    // block are hand-authored, not setting-derived. They must
    // be editable and have no `owner_setting_path`.
    let temp = tempfile::tempdir().unwrap();
    let base_dir = temp.path().join("base");
    let user_dir = temp.path().join("user");
    fs::create_dir_all(&base_dir).unwrap();

    fs::write(
        base_dir.join("strict.toml"),
        r#"
version = 1
id = "strict"
name = "Strict"
best_for = "Strict."
profile_type = "coding"

[security.rules.http.block_secret]
on = "http.request"
if = "request.data.contains_secret"
decision = "block"
priority = 5
"#,
    )
    .unwrap();
    let roots = test_roots(base_dir, user_dir);
    let effective = resolve_effective_vm_settings(&roots, Some("strict")).unwrap();
    let hand_authored = effective
        .rules
        .iter()
        .find(|rule| rule.id == "http.block_secret")
        .expect("hand-authored rule present");
    assert!(hand_authored.editable);
    assert!(hand_authored.owner_setting_path.is_none());
    assert!(hand_authored.owner_setting_label.is_none());
}

#[test]
fn vm_effective_settings_with_owned_rule_round_trips_through_disk() {
    // Slice 6b.1: the new fields must round-trip through the
    // on-disk vm-effective-settings.toml without surprising
    // existing readers. Backward-compat: existing files
    // without owner_* / editable fields still parse via
    // serde(default).
    let temp = tempfile::tempdir().unwrap();
    let base_dir = temp.path().join("base");
    let user_dir = temp.path().join("user");
    fs::create_dir_all(&base_dir).unwrap();
    let roots = test_roots(base_dir, user_dir);
    let effective = resolve_effective_vm_settings(&roots, None).unwrap();
    write_vm_effective_settings(temp.path(), &effective).unwrap();
    let reloaded = load_vm_effective_settings(temp.path()).unwrap();
    assert_eq!(effective, reloaded);
}

#[test]
fn profile_rule_rejects_priority_above_upper_bound() {
    let error = Profile::from_toml_str(
        r#"
id = "p"
name = "P"
best_for = "P"

[security.rules.http.too_high]
on = "http.request"
if = "true"
decision = "allow"
priority = 1001
"#,
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("priority must be in [-1000, 1000]"),
        "got: {error}"
    );
}

#[test]
fn profile_rule_rejects_priority_below_lower_bound() {
    let error = Profile::from_toml_str(
        r#"
id = "p"
name = "P"
best_for = "P"

[security.rules.http.too_low]
on = "http.request"
if = "true"
decision = "allow"
priority = -1001
"#,
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("priority must be in [-1000, 1000]"),
        "got: {error}"
    );
}

#[test]
fn profile_rule_rejects_reserved_catch_all_priority() {
    let error = Profile::from_toml_str(
        r#"
id = "p"
name = "P"
best_for = "P"

[security.rules.http.manual_catch_all]
on = "http.request"
if = "true"
decision = "allow"
priority = 1000
"#,
    )
    .unwrap_err();
    assert!(
        error.to_string().contains("priority 1000 is reserved"),
        "got: {error}"
    );
}

#[test]
fn discover_profiles_rejects_corp_priority_in_user_profile() {
    let temp = tempfile::tempdir().unwrap();
    let base_dir = temp.path().join("base");
    let user_dir = temp.path().join("user");
    fs::create_dir_all(&base_dir).unwrap();
    fs::create_dir_all(&user_dir).unwrap();
    fs::write(
        user_dir.join("usurper.toml"),
        r#"
version = 1
id = "usurper"
name = "Usurper"
best_for = "Trying to write corp-tier rules"
profile_type = "coding"

[security.rules.http.shadow_corp]
on = "http.request"
if = "true"
decision = "block"
priority = -500
"#,
    )
    .unwrap();
    let mut roots = test_roots(base_dir, user_dir);
    roots.allow_user_profiles = true;
    let error = discover_profiles(&roots).unwrap_err();
    assert!(
        error.to_string().contains("corp-exclusive"),
        "expected corp-exclusive violation, got: {error}"
    );
}

#[test]
fn discover_profiles_accepts_corp_priority_in_corp_profile() {
    // Same payload as the user-profile test but placed in a
    // corp_dirs directory: should pass.
    let temp = tempfile::tempdir().unwrap();
    let base_dir = temp.path().join("base");
    let user_dir = temp.path().join("user");
    let corp_dir = temp.path().join("corp");
    fs::create_dir_all(&base_dir).unwrap();
    fs::create_dir_all(&corp_dir).unwrap();
    fs::write(
        corp_dir.join("baseline.toml"),
        r#"
version = 1
id = "baseline"
name = "Baseline"
best_for = "Corp-tier rules"
profile_type = "coding"

[security.rules.http.org_default]
on = "http.request"
if = "true"
decision = "block"
priority = -500
"#,
    )
    .unwrap();
    let mut roots = test_roots(base_dir, user_dir);
    roots.corp_dirs = vec![corp_dir];
    discover_profiles(&roots).unwrap();
}

#[test]
fn corp_directive_rejects_rule_priority_outside_corp_range() {
    // Corp directives that try to author at user-tier priority
    // (1..999) are rejected -- corp authoritative tier is
    // [-1000, 0].
    let mut profile = Profile::everyday_work();
    let directive: CorpDirective = toml::from_str(
        r#"
operation = "add"
path = "security.rules.http.user_tier_attempt"
[value]
on = "http.request"
if = "true"
decision = "block"
priority = 50
"#,
    )
    .unwrap();
    let mut trace = ResolverTrace::new();
    let error = apply_corp_directives(&mut profile, &[directive], &mut trace).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("corp directive rule priority must be in [-1000, 0]"),
        "got: {error}"
    );
}

#[test]
fn corp_directive_rejects_catch_all_priority() {
    let mut profile = Profile::everyday_work();
    let directive: CorpDirective = toml::from_str(
        r#"
operation = "add"
path = "security.rules.http.catch_all_attempt"
[value]
on = "http.request"
if = "true"
decision = "block"
priority = 1000
"#,
    )
    .unwrap();
    let mut trace = ResolverTrace::new();
    let error = apply_corp_directives(&mut profile, &[directive], &mut trace).unwrap_err();
    // The catch-all reservation fires first inside
    // ProfileRule::validate during parse, before the
    // corp-range check.
    assert!(
        error.to_string().contains("priority 1000 is reserved")
            || error.to_string().contains("corp directive rule priority"),
        "got: {error}"
    );
}

#[test]
fn nested_rules_under_ai_provider_host_emit_with_owner_setting_path() {
    // Slice 6b.3: rules authored under `ai.providers.<name>`
    // flow into effective rules tagged with the host's path so
    // callers know "this rule lives with the openai provider
    // config." They remain editable -- the owner is for
    // semantic clarity, not the mutation gate.
    let temp = tempfile::tempdir().unwrap();
    let base_dir = temp.path().join("base");
    let corp_dir = temp.path().join("corp");
    let user_dir = temp.path().join("user");
    fs::create_dir_all(&base_dir).unwrap();
    fs::create_dir_all(&corp_dir).unwrap();
    fs::write(
        corp_dir.join("with-nested.toml"),
        r#"
version = 1
id = "with-nested"
name = "With Nested"
best_for = "Corp profile with nested provider rules"
profile_type = "coding"

[ai.providers.openai]
enabled = true
base_url = "https://api.openai.com"

[ai.providers.openai.rules.http.allow_api]
on = "http.request"
if = "true"
decision = "allow"
priority = -10
"#,
    )
    .unwrap();

    let mut roots = test_roots(base_dir, user_dir);
    roots.corp_dirs = vec![corp_dir];
    let effective = resolve_effective_vm_settings(&roots, Some("with-nested")).unwrap();
    let nested = effective
        .rules
        .iter()
        .find(|rule| rule.id == "http.allow_api")
        .expect("nested rule must surface in effective rules");
    assert_eq!(
        nested.owner_setting_path.as_deref(),
        Some("ai.providers.openai"),
    );
    assert_eq!(
        nested.owner_setting_label.as_deref(),
        Some("AI provider · openai"),
    );
    assert!(
        nested.editable,
        "nested rules remain editable; only setting-derived rules are uneditable"
    );
    assert_eq!(nested.decision, RuleDecision::Allow);
}

#[test]
fn nested_rules_under_mcp_connector_host_emit_with_owner_setting_path() {
    let temp = tempfile::tempdir().unwrap();
    let base_dir = temp.path().join("base");
    let corp_dir = temp.path().join("corp");
    let user_dir = temp.path().join("user");
    fs::create_dir_all(&base_dir).unwrap();
    fs::create_dir_all(&corp_dir).unwrap();
    fs::write(
        corp_dir.join("with-nested.toml"),
        r#"
version = 1
id = "with-nested"
name = "With Nested"
best_for = "Corp profile with nested connector rules"
profile_type = "coding"

[mcpServers.github]
enabled = true
command = "npx"

[mcpServers.github.capsem.rules.mcp.allow_repo_read]
on = "mcp.request"
if = "true"
decision = "allow"
priority = -10
"#,
    )
    .unwrap();

    let mut roots = test_roots(base_dir, user_dir);
    roots.corp_dirs = vec![corp_dir];
    let effective = resolve_effective_vm_settings(&roots, Some("with-nested")).unwrap();
    let nested = effective
        .rules
        .iter()
        .find(|rule| rule.id == "mcp.allow_repo_read")
        .expect("nested rule must surface");
    assert_eq!(
        nested.owner_setting_path.as_deref(),
        Some("mcpServers.github.capsem"),
    );
    assert_eq!(
        nested.owner_setting_label.as_deref(),
        Some("MCP server · github"),
    );
}

#[test]
fn empty_nested_rule_block_round_trips_through_disk() {
    // Backward-compat guard: existing profile TOML files without
    // [ai.providers.openai.rules.*] sections must parse cleanly.
    // The serde(skip_serializing_if = "is_empty") attribute keeps
    // the on-disk shape unchanged when no nested rules exist.
    let profile = Profile::from_toml_str(
        r#"
id = "p"
name = "P"
best_for = "P"

[ai.providers.openai]
enabled = true
"#,
    )
    .unwrap();
    assert!(profile.ai.providers["openai"].rules.is_empty());
    let toml = toml::to_string(&profile).unwrap();
    assert!(
        !toml.contains("[ai.providers.openai.rules"),
        "empty nested rules should not serialize"
    );
}

#[test]
fn catch_all_rules_land_at_priority_1000_per_runtime_callback() {
    // Slice 6b.5: one catch-all per runtime callback at the
    // reserved priority. With network_egress = "ask" (default),
    // dns/http/model catch-alls decision = Ask; mcp.default
    // decision = Ask (from mcp_tools default).
    let temp = tempfile::tempdir().unwrap();
    let base_dir = temp.path().join("base");
    let user_dir = temp.path().join("user");
    fs::create_dir_all(&base_dir).unwrap();
    let roots = test_roots(base_dir, user_dir);
    let effective = resolve_effective_vm_settings(&roots, None).unwrap();

    let expected = [
        ("dns.default", "dns.request"),
        ("http.default_read", "http.read"),
        ("http.default_write", "http.write"),
        ("model.default", "model.request"),
        ("mcp.default", "mcp.request"),
    ];
    for (id, callback) in expected {
        let rule = effective
            .rules
            .iter()
            .find(|rule| rule.id == id)
            .unwrap_or_else(|| panic!("missing catch-all '{id}'"));
        assert_eq!(rule.priority, RULE_CATCH_ALL_PRIORITY);
        assert_eq!(rule.callback, callback);
        assert_eq!(rule.condition, "true");
        assert!(rule.derived);
        assert!(!rule.editable);
    }
}

#[test]
fn http_catch_all_split_follows_capability_network_egress() {
    // network_egress = Block flips http.default_read and
    // http.default_write decisions to Block; flipping to Allow
    // flips them back. Locks the "read/write share the same
    // capability" contract.
    let temp = tempfile::tempdir().unwrap();
    let base_dir = temp.path().join("base");
    let user_dir = temp.path().join("user");
    fs::create_dir_all(&base_dir).unwrap();
    let roots = test_roots(base_dir, user_dir);
    let mut profile = profile_value("strict", "Strict", ProfileType::Coding);
    profile.security.capabilities.network_egress = CapabilityMode::Block;
    create_user_profile(&roots, profile).unwrap();

    let effective = resolve_effective_vm_settings(&roots, Some("strict")).unwrap();
    for id in ["http.default_read", "http.default_write"] {
        let rule = effective.rules.iter().find(|rule| rule.id == id).unwrap();
        assert_eq!(rule.decision, RuleDecision::Block, "{id} blocks");
    }
}

#[test]
fn mcp_catch_all_follows_capability_mcp_tools() {
    let temp = tempfile::tempdir().unwrap();
    let base_dir = temp.path().join("base");
    let user_dir = temp.path().join("user");
    fs::create_dir_all(&base_dir).unwrap();
    let roots = test_roots(base_dir, user_dir);
    let mut profile = profile_value("strict", "Strict", ProfileType::Coding);
    profile.security.capabilities.mcp_tools = CapabilityMode::Block;
    create_user_profile(&roots, profile).unwrap();

    let effective = resolve_effective_vm_settings(&roots, Some("strict")).unwrap();
    let mcp_rule = effective
        .rules
        .iter()
        .find(|rule| rule.id == "mcp.default")
        .unwrap();
    assert_eq!(mcp_rule.decision, RuleDecision::Block);
    assert_eq!(
        mcp_rule.owner_setting_path.as_deref(),
        Some("security.capabilities.mcp_tools")
    );
}

#[test]
fn provider_toggle_enabled_emits_allow_rule_at_priority_zero() {
    // Slice 6b.6: ai.providers.openai.enabled = true emits
    // allow rules at priority 0 for api.openai.com on both
    // dns and http callbacks. Rule owner points at the
    // enabled toggle; editable = false.
    let temp = tempfile::tempdir().unwrap();
    let base_dir = temp.path().join("base");
    let corp_dir = temp.path().join("corp");
    let user_dir = temp.path().join("user");
    fs::create_dir_all(&base_dir).unwrap();
    fs::create_dir_all(&corp_dir).unwrap();
    fs::write(
        corp_dir.join("with-openai.toml"),
        r#"
version = 1
id = "with-openai"
name = "OpenAI On"
best_for = "Corp profile enabling OpenAI"
profile_type = "coding"

[ai.providers.openai]
enabled = true
"#,
    )
    .unwrap();

    let mut roots = test_roots(base_dir, user_dir);
    roots.corp_dirs = vec![corp_dir];
    let effective = resolve_effective_vm_settings(&roots, Some("with-openai")).unwrap();
    let dns_allow = effective
        .rules
        .iter()
        .find(|rule| rule.id == "dns.provider_openai_allow_api-openai-com")
        .expect("dns allow for api.openai.com expected");
    assert_eq!(dns_allow.priority, 0);
    assert_eq!(dns_allow.decision, RuleDecision::Allow);
    assert_eq!(dns_allow.callback, "dns.request");
    assert_eq!(
        dns_allow.owner_setting_path.as_deref(),
        Some("ai.providers.openai.enabled")
    );
    assert!(!dns_allow.editable);
    assert!(effective.rules.iter().any(|rule| rule.id
        == "http.provider_openai_allow_api-openai-com"
        && rule.decision == RuleDecision::Allow));
}

#[test]
fn provider_toggle_disabled_emits_block_rule_at_priority_zero() {
    let temp = tempfile::tempdir().unwrap();
    let base_dir = temp.path().join("base");
    let corp_dir = temp.path().join("corp");
    let user_dir = temp.path().join("user");
    fs::create_dir_all(&base_dir).unwrap();
    fs::create_dir_all(&corp_dir).unwrap();
    fs::write(
        corp_dir.join("openai-off.toml"),
        r#"
version = 1
id = "openai-off"
name = "OpenAI Off"
best_for = "Corp profile blocking OpenAI"
profile_type = "coding"

[ai.providers.openai]
enabled = false
"#,
    )
    .unwrap();

    let mut roots = test_roots(base_dir, user_dir);
    roots.corp_dirs = vec![corp_dir];
    let effective = resolve_effective_vm_settings(&roots, Some("openai-off")).unwrap();
    let dns_block = effective
        .rules
        .iter()
        .find(|rule| rule.id == "dns.provider_openai_block_api-openai-com")
        .expect("dns block for api.openai.com expected");
    assert_eq!(dns_block.priority, 0);
    assert_eq!(dns_block.decision, RuleDecision::Block);
    assert!(!dns_block.editable);
}

#[test]
fn provider_toggle_uses_base_url_host_for_unknown_provider() {
    // Slice 6b.6: unknown provider ids fall back to deriving
    // the host from base_url. This lets corps onboard
    // self-hosted endpoints without us hardcoding their host.
    let temp = tempfile::tempdir().unwrap();
    let base_dir = temp.path().join("base");
    let corp_dir = temp.path().join("corp");
    let user_dir = temp.path().join("user");
    fs::create_dir_all(&base_dir).unwrap();
    fs::create_dir_all(&corp_dir).unwrap();
    fs::write(
        corp_dir.join("custom.toml"),
        r#"
version = 1
id = "custom"
name = "Custom"
best_for = "Self-hosted model endpoint"
profile_type = "coding"

[ai.providers.local-llm]
enabled = true
base_url = "https://llm.internal.corp:8443/v1"
"#,
    )
    .unwrap();

    let mut roots = test_roots(base_dir, user_dir);
    roots.corp_dirs = vec![corp_dir];
    let effective = resolve_effective_vm_settings(&roots, Some("custom")).unwrap();
    assert!(
        effective
            .rules
            .iter()
            .any(|rule| rule.id == "dns.provider_local-llm_allow_llm-internal-corp"),
        "should derive host from base_url; got rules: {:?}",
        effective
            .rules
            .iter()
            .map(|r| r.id.clone())
            .collect::<Vec<_>>()
    );
}

#[test]
fn mcp_allowed_tools_emits_allow_rule_per_tool_at_priority_zero() {
    // Slice 6b.7: mcpServers.<name>.capsem.allowed_tools emits
    // one allow rule per tool at priority 0, condition
    // `tool.name == '<tool>'`, owner pointing at the
    // allowed_tools list.
    let temp = tempfile::tempdir().unwrap();
    let base_dir = temp.path().join("base");
    let corp_dir = temp.path().join("corp");
    let user_dir = temp.path().join("user");
    fs::create_dir_all(&base_dir).unwrap();
    fs::create_dir_all(&corp_dir).unwrap();
    fs::write(
        corp_dir.join("github-connector.toml"),
        r#"
version = 1
id = "github-connector"
name = "GitHub Connector"
best_for = "Corp profile with GitHub tools allowlist"
profile_type = "coding"

[mcpServers.github]
enabled = true
command = "npx"
[mcpServers.github.capsem]
allowed_tools = ["repo.read", "issue.write"]
"#,
    )
    .unwrap();

    let mut roots = test_roots(base_dir, user_dir);
    roots.corp_dirs = vec![corp_dir];
    let effective = resolve_effective_vm_settings(&roots, Some("github-connector")).unwrap();

    for (expected_id, expected_tool) in [
        ("mcp.connector_github_allow_repo-read", "repo.read"),
        ("mcp.connector_github_allow_issue-write", "issue.write"),
    ] {
        let rule = effective
            .rules
            .iter()
            .find(|rule| rule.id == expected_id)
            .unwrap_or_else(|| panic!("expected derived rule '{expected_id}'"));
        assert_eq!(rule.priority, 0);
        assert_eq!(rule.decision, RuleDecision::Allow);
        assert!(rule.condition.contains(expected_tool));
        assert_eq!(
            rule.owner_setting_path.as_deref(),
            Some("mcpServers.github.capsem.allowed_tools")
        );
        assert!(!rule.editable);
    }
}

#[test]
fn ensure_rule_editable_allows_hand_authored_rules() {
    let temp = tempfile::tempdir().unwrap();
    let base_dir = temp.path().join("base");
    let user_dir = temp.path().join("user");
    fs::create_dir_all(&base_dir).unwrap();
    fs::write(
        base_dir.join("strict.toml"),
        r#"
version = 1
id = "strict"
name = "Strict"
best_for = "Strict."
profile_type = "coding"

[security.rules.http.block_secret]
on = "http.request"
if = "request.data.contains_secret"
decision = "block"
priority = 5
"#,
    )
    .unwrap();
    let roots = test_roots(base_dir, user_dir);
    let effective = resolve_effective_vm_settings(&roots, Some("strict")).unwrap();
    let rule = effective
        .rules
        .iter()
        .find(|rule| rule.id == "http.block_secret")
        .unwrap();
    ensure_rule_editable(rule).unwrap();
}

#[test]
fn ensure_rule_editable_refuses_catch_all_rules() {
    let temp = tempfile::tempdir().unwrap();
    let base_dir = temp.path().join("base");
    let user_dir = temp.path().join("user");
    fs::create_dir_all(&base_dir).unwrap();
    let roots = test_roots(base_dir, user_dir);
    let effective = resolve_effective_vm_settings(&roots, None).unwrap();
    let catch_all = effective
        .rules
        .iter()
        .find(|rule| rule.id == "http.default_read")
        .unwrap();
    let error = ensure_rule_editable(catch_all).unwrap_err();
    assert!(
        matches!(
            error,
            SettingsProfilesError::RuleManagedBySetting { ref owner_setting_path, .. }
                if owner_setting_path == "security.capabilities.network_egress"
        ),
        "got {error:?}"
    );
    let msg = error.to_string();
    assert!(msg.contains("managed by setting"));
    assert!(msg.contains("security.capabilities.network_egress"));
}

#[test]
fn ensure_rule_editable_refuses_provider_toggle_rules() {
    let temp = tempfile::tempdir().unwrap();
    let base_dir = temp.path().join("base");
    let corp_dir = temp.path().join("corp");
    let user_dir = temp.path().join("user");
    fs::create_dir_all(&base_dir).unwrap();
    fs::create_dir_all(&corp_dir).unwrap();
    fs::write(
        corp_dir.join("openai-on.toml"),
        r#"
version = 1
id = "openai-on"
name = "OpenAI On"
best_for = "Corp profile enabling OpenAI"
profile_type = "coding"

[ai.providers.openai]
enabled = true
"#,
    )
    .unwrap();
    let mut roots = test_roots(base_dir, user_dir);
    roots.corp_dirs = vec![corp_dir];
    let effective = resolve_effective_vm_settings(&roots, Some("openai-on")).unwrap();
    let provider_rule = effective
        .rules
        .iter()
        .find(|rule| rule.id == "dns.provider_openai_allow_api-openai-com")
        .unwrap();
    let error = ensure_rule_editable(provider_rule).unwrap_err();
    assert!(matches!(
        error,
        SettingsProfilesError::RuleManagedBySetting { ref owner_setting_path, .. }
            if owner_setting_path == "ai.providers.openai.enabled"
    ));
}
