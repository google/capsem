use super::*;

// -----------------------------------------------------------------------
// retired callback policy compatibility
// -----------------------------------------------------------------------

#[test]
fn settings_file_rejects_old_policy_tables() {
    let old_table = "policy".to_string() + ".http.block_openai_github";
    let error = toml::from_str::<SettingsFile>(
        r#"
[__OLD_TABLE__]
on = "http.request"
if = 'http.host == "github.com"'
decision = "block"
priority = 10
"#
        .replace("__OLD_TABLE__", &old_table)
        .as_str(),
    )
    .expect_err("old policy tables must not deserialize");

    assert!(
        error.to_string().contains("unknown field") || error.to_string().contains("policy"),
        "{error}"
    );
}

#[test]
fn batch_update_settings_json_rejects_old_policy_rule_shape_atomically() {
    with_temp_configs(vec![], vec![], |user_path, _| {
        let mut changes = HashMap::new();
        let retired_key = "policy".to_string() + ".http.block_openai_github";
        changes.insert("appearance.dark_mode".to_string(), serde_json::json!(true));
        changes.insert(
            retired_key.clone(),
            serde_json::json!({
                "on": "http.request",
                "if": "http.host == 'github.com'",
                "decision": "block",
                "priority": 10
            }),
        );

        let error = loader::batch_update_settings_json(&changes).expect_err("old policy writes must reject");
        assert!(error.contains(&format!("unknown setting: {retired_key}")), "{error}");
        let loaded = loader::load_settings_file(user_path).unwrap();
        assert!(
            loaded.settings.is_empty(),
            "batch rejection must leave settings.toml unchanged"
        );
    });
}

#[test]
fn settings_file_parses_provider_security_rules_under_ai_provider_sections() {
    let file: SettingsFile = toml::from_str(
        r#"
[ai.openai]
name = "OpenAI"
protocol = "openai"
url = "https://api.openai.com/v1"

[ai.openai.rules.http_api]
name = "openai_http_api_observed"
action = "allow"
detection_level = "informational"
match = 'http.host.matches("(^|.*\.)openai\.com$")'
"#,
    )
    .expect("provider security rules parse inside settings file");

    assert!(file.ai.contains_key("openai"));
    let rules = ProviderRuleProfile { ai: file.ai.clone() }
        .compile_rule_set(SecurityRuleSource::User)
        .expect("provider security rules compile");
    assert!(rules
        .rules()
        .iter()
        .any(|rule| rule.rule_id == "profiles.rules.ai_openai_http_api"));

    let policies = MergedPolicies::from_files(&file, &SettingsFile::default()).expect("policies merge");
    assert!(policies
        .security_rules
        .rules()
        .iter()
        .any(|rule| rule.rule_id == "profiles.rules.ai_openai_http_api"));
}

#[test]
fn settings_file_parses_discovery_only_provider_record() {
    let file: SettingsFile = toml::from_str(
        r#"
[ai.openai.discovery]
observed_at = "2026-06-06T10:00:00Z"
source = "http.header.authorization"
event_type = "http.request"
confidence = 1.0
credential_ref = "credential:blake3:0000000000000000000000000000000000000000000000000000000000000000"
trace_id = "trace-openai"
"#,
    )
    .expect("discovery-only provider records are valid settings TOML");

    let discovery = file.ai["openai"].discovery.as_ref().unwrap();
    assert_eq!(discovery.event_type.as_deref(), Some("http.request"));
    assert_eq!(
        discovery.credential_ref.as_deref(),
        Some("credential:blake3:0000000000000000000000000000000000000000000000000000000000000000")
    );

    let policies = MergedPolicies::from_files(&file, &SettingsFile::default()).expect("policies merge");
    assert_eq!(
        policies.model_endpoints.protocol_for_host("api.openai.com"),
        Some(crate::net::ai_traffic::provider::ModelProtocol::OpenAi)
    );
    assert!(policies
        .security_rules
        .rules()
        .iter()
        .any(|rule| rule.rule_id == "profiles.rules.ai_openai_http_api"));
}

#[test]
fn provider_discovery_rejects_unknown_event_type_and_raw_secret_reference() {
    let stale_event_type = toml::from_str::<SettingsFile>(
        r#"
[ai.openai.discovery]
observed_at = "2026-06-06T10:00:00Z"
source = "old-observer"
event_type = "mcp.request"
confidence = 1.0
"#,
    )
    .expect("serde accepts the shape before provider validation");
    let profile = ProviderRuleProfile {
        ai: stale_event_type.ai,
    };
    assert!(
        profile.validate().is_err(),
        "provider discovery must use canonical runtime event types"
    );

    let raw_secret = toml::from_str::<SettingsFile>(
        r#"
[ai.openai.discovery]
observed_at = "2026-06-06T10:00:00Z"
source = "old-observer"
event_type = "http.request"
confidence = 1.0
credential_ref = "sk-raw-secret"
"#,
    )
    .expect("serde accepts the shape before provider validation");
    let profile = ProviderRuleProfile { ai: raw_secret.ai };
    assert!(
        profile.validate().is_err(),
        "provider discovery must never accept raw credentials"
    );
}

#[test]
fn tool_config_sources_are_rejected_from_settings_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.toml");
    std::fs::write(
        &path,
        r#"
[tool_config_sources.codex_config]
tool_id = "codex"
guest_path = "/root/.codex/config.toml"
format = "toml"
observed_hash = "blake3:0000000000000000000000000000000000000000000000000000000000000000"
observed_version = "2026-06-06"
inferred_endpoint_ref = "ai.openai"
credential_refs = ["credential:blake3:1111111111111111111111111111111111111111111111111111111111111111"]
allowed_overlays = ["mcp_injection", "broker_placeholders"]
"#,
    )
    .unwrap();

    let error = load_settings_file(&path).expect_err("tool_config_sources is runtime evidence");
    assert!(error.contains("tool_config_sources"), "{error}");
}

#[test]
fn tool_config_sources_are_not_a_static_credential_escape_hatch() {
    let cases = [
        (
            "raw credential ref",
            r#"
[tool_config_sources.codex_config]
tool_id = "codex"
guest_path = "/root/.codex/config.toml"
format = "toml"
credential_refs = ["sk-raw-secret"]
"#,
        ),
        (
            "rendered content field",
            r#"
[tool_config_sources.codex_config]
tool_id = "codex"
guest_path = "/root/.codex/config.toml"
format = "toml"
content = "api_key = 'sk-raw-secret'"
"#,
        ),
        (
            "bad hash",
            r#"
[tool_config_sources.codex_config]
tool_id = "codex"
guest_path = "/root/.codex/config.toml"
format = "toml"
observed_hash = "abc123"
"#,
        ),
        (
            "bad endpoint ref",
            r#"
[tool_config_sources.codex_config]
tool_id = "codex"
guest_path = "/root/.codex/config.toml"
format = "toml"
inferred_endpoint_ref = "openai"
"#,
        ),
    ];

    for (name, toml_text) in cases {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        std::fs::write(&path, toml_text).unwrap();
        let error = load_settings_file(&path).expect_err("tool_config_sources is retired");
        assert!(error.contains("tool_config_sources"), "{name}: {error}");
    }
}

#[test]
fn settings_loader_rejects_raw_provider_credentials_but_accepts_broker_refs() {
    let dir = tempfile::tempdir().unwrap();
    let valid_path = dir.path().join("valid.toml");
    std::fs::write(
        &valid_path,
        r#"
[settings]
"repository.providers.github.token" = { value = "", modified = "2026-06-06T10:00:00Z" }
"#,
    )
    .unwrap();
    let valid_result = load_settings_file(&valid_path);
    assert!(
        valid_result.is_ok(),
        "broker refs and empty credential settings are allowed: {valid_result:?}"
    );

    let raw_path = dir.path().join("raw.toml");
    std::fs::write(
        &raw_path,
        r#"
[settings]
"ai.openai.api_key" = { value = "sk-raw-openai", modified = "2026-06-06T10:00:00Z" }
"#,
    )
    .unwrap();
    let error = load_settings_file(&raw_path).expect_err("raw provider credential must fail");
    assert!(
        error.contains("retired AI setting id ai.openai.api_key"),
        "error should reject retired AI setting ids: {error}"
    );
}

#[test]
fn batch_update_settings_rejects_raw_provider_credentials_atomically() {
    with_temp_configs(vec![], vec![], |user_path, _| {
        let mut changes = HashMap::new();
        changes.insert("ai.openai.api_key".to_string(), serde_json::json!("sk-raw-openai"));

        let result = loader::batch_update_settings_json(&changes);
        let error = result.expect_err("retired API key writes must be rejected");
        assert!(error.contains("unknown setting"), "{error}");
        let loaded = loader::load_settings_file(user_path).unwrap();
        assert!(
            !loaded.settings.contains_key("ai.openai.api_key"),
            "raw rejected setting must not be written"
        );
    });
}

#[test]
fn builtin_provider_rules_compile_only_into_security_rules() {
    let policies =
        MergedPolicies::from_files(&SettingsFile::default(), &SettingsFile::default()).expect("policies merge");
    let rule_ids = policies
        .security_rules
        .rules()
        .iter()
        .map(|rule| rule.rule_id.as_str())
        .collect::<Vec<_>>();

    assert!(rule_ids.contains(&"profiles.rules.ai_openai_http_api"));
    assert!(rule_ids.contains(&"profiles.rules.ai_ollama_http_local_host"));
    assert!(rule_ids.contains(&"profiles.rules.ai_google_dns_googleapis"));
    assert!(
        rule_ids.iter().all(|id| !id.starts_with("policy.")),
        "provider rules must not be mirrored into the retired callback policy rail"
    );
}

#[test]
fn merged_policies_compile_profile_and_corp_security_rules() {
    let user = SettingsFile {
        profiles: SecurityRuleProfile::parse_toml(
            r#"
[profiles.rules.skill_loaded]
name = "skill_loaded"
action = "allow"
detection_level = "informational"
match = 'file.read.path.contains("skills/")'
"#,
        )
        .unwrap()
        .profiles,
        ..Default::default()
    };
    let corp = SettingsFile {
        corp: SecurityRuleProfile::parse_toml(
            r#"
[corp.rules.block_openai]
name = "block_openai"
action = "block"
detection_level = "critical"
match = 'http.host.matches("(^|.*\.)openai\.com$")'
"#,
        )
        .unwrap()
        .corp,
        ..Default::default()
    };

    let policies = MergedPolicies::from_files(&user, &corp).expect("policies merge");
    let ids: Vec<_> = policies
        .security_rules
        .rules()
        .iter()
        .map(|rule| (rule.rule_id.as_str(), rule.priority))
        .collect();

    assert!(ids.contains(&("profiles.rules.skill_loaded", 10)));
    assert!(ids.contains(&("corp.rules.block_openai", -10)));
}

#[test]
fn integration_corp_rule_beats_profile_default_allow_for_deny_target() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("capsem-core lives under crates/");
    let _guard = crate::credential_broker::TEST_ENV_LOCK.blocking_lock();
    let capsem_home = tempfile::tempdir().unwrap();
    std::fs::copy(
        root.join("tests/fixtures/config/integration/settings.toml"),
        capsem_home.path().join("settings.toml"),
    )
    .unwrap();
    let _settings_home = EnvVarGuard::set("CAPSEM_HOME", capsem_home.path());
    let _corp_config = EnvVarGuard::set(
        "CAPSEM_CORP_CONFIG",
        root.join("tests/fixtures/config/integration/corp.toml"),
    );
    let (user, corp) = load_settings_and_corp_files();
    let policies = MergedPolicies::from_files(&user, &corp).expect("policies merge");
    let event = serde_json::json!({
        "http": {
            "host": "127.0.0.1",
            "path": "/deny-target"
        }
    });
    let evaluation = policies
        .security_rules
        .evaluate(&event)
        .expect("integration event evaluates");
    let enforcement_rules: Vec<_> = evaluation
        .enforcement_rules()
        .into_iter()
        .map(|rule| (rule.rule_id.as_str(), rule.action, rule.priority))
        .collect();

    assert_eq!(
        enforcement_rules.first(),
        Some(&("corp.rules.block_local_deny_target", SecurityRuleAction::Block, -100)),
        "corp block must be the first enforcement decision before profile defaults: {enforcement_rules:?}"
    );
}

#[test]
fn merged_policies_carry_live_model_endpoint_registry() {
    let user: SettingsFile = toml::from_str(
        r#"
[ai.private_gateway]
name = "Private Gateway"
protocol = "openai-compatible"
url = "https://llm.internal.example/v1"
listen_ports = [443, 8443]
allowed_remote_targets = ["llm.internal.example:443", "company-openai:8443"]

[ai.private_gateway.rules.http_api]
name = "private_gateway_http_seen"
action = "allow"
match = 'http.host == "llm.internal.example"'
"#,
    )
    .expect("settings parse");

    let policies = MergedPolicies::from_files(&user, &SettingsFile::default()).expect("policies merge");

    assert_eq!(
        policies.model_endpoints.protocol_for_host("llm.internal.example"),
        Some(crate::net::ai_traffic::provider::ModelProtocol::OpenAi)
    );
    assert_eq!(
        policies.model_endpoints.protocol_for_host("api.openai.com"),
        Some(crate::net::ai_traffic::provider::ModelProtocol::OpenAi)
    );
    assert_eq!(
        policies.model_endpoints.protocol_for_target("company-openai", 8443),
        Some(crate::net::ai_traffic::provider::ModelProtocol::OpenAi)
    );
    assert_eq!(
        policies.model_endpoints.protocol_for_target("company-openai", 11434),
        None
    );
    let endpoint = policies
        .model_endpoints
        .get("private_gateway")
        .expect("private endpoint");
    assert_eq!(endpoint.provider_id, "private_gateway");
    assert_eq!(
        endpoint.allowed_remote_targets,
        vec!["llm.internal.example:443", "company-openai:8443"]
    );
}

#[test]
fn load_settings_file_merges_referenced_sigma_into_security_rules() {
    let dir = tempfile::tempdir().unwrap();
    let settings_path = dir.path().join("settings.toml");
    std::fs::write(
        dir.path().join("detection.yaml"),
        r#"
title: OpenAI Traffic To Unexpected Endpoint
id: 11111111-1111-4111-8111-111111111111
logsource:
  product: capsem
  service: security_event
detection:
  selection_model:
    model.provider: openai
  filter_approved_endpoint:
    http.host: api.openai.com
  condition: selection_model and not filter_approved_endpoint
level: high
capsem:
  action: block
"#,
    )
    .unwrap();
    std::fs::write(
        &settings_path,
        r#"
[rule_files]
sigma = "detection.yaml"
"#,
    )
    .unwrap();

    let user = load_settings_file(&settings_path).expect("settings load");
    let policies = MergedPolicies::from_files(&user, &SettingsFile::default()).expect("policies merge");
    let rule = policies
        .security_rules
        .rules()
        .iter()
        .find(|rule| rule.rule_id == "profiles.rules.openai_traffic_to_unexpected_endpoint")
        .expect("referenced Sigma rule compiles into runtime rules");

    assert_eq!(rule.action, SecurityRuleAction::Block);
    assert_eq!(rule.detection_level, Some(DetectionLevel::High));
}

#[test]
fn provider_security_rules_merge_corp_block_with_rule_priority() {
    let corp: SettingsFile = toml::from_str(
        r#"
[ai.openai]
name = "OpenAI"
protocol = "openai"
url = "https://api.openai.com/v1"

[ai.openai.rules.http_api]
name = "openai_http_api_corp_block"
action = "block"
detection_level = "critical"
priority = -100
corp_locked = true
reason = "OpenAI blocked by corporate policy"
match = 'http.host.matches("(^|.*\.)openai\.com$")'
"#,
    )
    .unwrap();

    let merged = ProviderRuleProfile::merge_defaults_user_and_corp(
        &ProviderRuleProfile::default(),
        &ProviderRuleProfile { ai: corp.ai },
    )
    .expect("provider rules merge");
    let rules = merged
        .compile_rule_set(SecurityRuleSource::Corp)
        .expect("merged provider rules compile");
    let rule = rules
        .rules()
        .iter()
        .find(|rule| rule.rule_id == "profiles.rules.ai_openai_http_api")
        .expect("corp provider rule exists");
    assert_eq!(rule.name, "openai_http_api_corp_block");
    assert_eq!(rule.action, SecurityRuleAction::Block);
    assert_eq!(rule.priority, -100);
    assert_eq!(rule.detection_level, Some(DetectionLevel::Critical));
}

#[test]
fn provider_discovery_and_user_allow_cannot_reenable_corp_blocked_provider() {
    let user: SettingsFile = toml::from_str(
        r#"
[ai.openai.discovery]
observed_at = "2026-06-06T10:00:00Z"
source = "http.header.authorization"
event_type = "http.request"
confidence = 1.0
credential_ref = "credential:blake3:0000000000000000000000000000000000000000000000000000000000000000"

[ai.openai.rules.http_api]
name = "openai_http_api_user_allow"
action = "allow"
priority = 100
match = 'http.host.matches("(^|.*\.)openai\.com$")'
"#,
    )
    .unwrap();
    let corp: SettingsFile = toml::from_str(
        r#"
[ai.openai.rules.http_api]
name = "openai_http_api_corp_block"
action = "block"
detection_level = "critical"
priority = -100
corp_locked = true
reason = "OpenAI blocked by corporate policy"
match = 'http.host.matches("(^|.*\.)openai\.com$")'
"#,
    )
    .unwrap();

    let policies = MergedPolicies::from_files(&user, &corp).expect("policies merge");
    let rule = policies
        .security_rules
        .rules()
        .iter()
        .find(|rule| rule.rule_id == "profiles.rules.ai_openai_http_api")
        .expect("provider rule id should exist");
    assert_eq!(rule.name, "openai_http_api_corp_block");
    assert_eq!(rule.action, SecurityRuleAction::Block);
    assert_eq!(rule.priority, -100);
    assert!(rule.corp_locked);

    let event = serde_json::json!({
        "http": {
            "host": "api.openai.com"
        }
    });
    let evaluation = policies
        .security_rules
        .evaluate(&event)
        .expect("security event evaluates");
    assert!(
        evaluation
            .rules_for_action(SecurityRuleAction::Allow)
            .iter()
            .all(|rule| rule.rule_id != "profiles.rules.ai_openai_http_api"),
        "user provider allow rule must be replaced by the corp block, not matched alongside it"
    );
    assert_eq!(
        evaluation.enforcement_rules()[0].rule_id,
        "profiles.rules.ai_openai_http_api"
    );
}

#[test]
fn load_settings_response_does_not_expose_provider_status() {
    let _guard = crate::credential_broker::TEST_ENV_LOCK.blocking_lock();

    let dir = tempfile::tempdir().unwrap();
    let user_path = dir.path().join("settings.toml");
    let corp_path = dir.path().join("corp.toml");
    std::fs::write(
        &user_path,
        r#"
[settings]
[ai.openai.discovery]
observed_at = "2026-06-06T10:00:00Z"
source = "http.header.authorization"
event_type = "http.request"
confidence = 1.0
credential_ref = "credential:blake3:0000000000000000000000000000000000000000000000000000000000000000"
"#,
    )
    .unwrap();
    std::fs::write(
        &corp_path,
        r#"
[ai.openai.rules.http_api]
name = "openai_http_api_corp_block"
action = "block"
priority = -100
corp_locked = true
match = 'http.host.matches("(^|.*\.)openai\.com$")'
"#,
    )
    .unwrap();
    let _settings_home = EnvVarGuard::set("CAPSEM_HOME", dir.path());
    let _corp_config = EnvVarGuard::set("CAPSEM_CORP_CONFIG", &corp_path);

    let serialized = serde_json::to_value(load_settings_response()).expect("settings response serializes");
    assert!(
        serialized.get("providers").is_none(),
        "settings response must not expose provider status"
    );
    assert!(
        serialized.get("tool_config_sources").is_none(),
        "settings response must not expose runtime tool config observations"
    );
    assert!(
        serialized.get("policy").is_none(),
        "settings response must not expose retired policy payloads"
    );
}

#[test]
fn load_settings_response_exposes_settings_tree_only() {
    let _guard = crate::credential_broker::TEST_ENV_LOCK.blocking_lock();

    let dir = tempfile::tempdir().unwrap();
    let user_path = dir.path().join("settings.toml");
    let corp_path = dir.path().join("corp.toml");
    write_settings_file(&user_path, &SettingsFile::default()).unwrap();
    write_settings_file(&corp_path, &SettingsFile::default()).unwrap();
    let _settings_home = EnvVarGuard::set("CAPSEM_HOME", dir.path());
    let _corp_config = EnvVarGuard::set("CAPSEM_CORP_CONFIG", &corp_path);

    let serialized = serde_json::to_value(load_settings_response()).expect("settings response serializes");
    assert!(
        serialized.get("tree").is_some(),
        "settings response must expose the settings tree"
    );
    assert!(
        serialized.get("issues").is_some(),
        "settings response must expose config issues"
    );
    let tree = serialized.get("tree").expect("settings tree is present").to_string();
    assert!(
        !tree.contains("\"mcp\"") && !tree.contains("MCP Servers"),
        "settings response must not expose profile-owned MCP configuration"
    );
    assert!(
        serialized.get("providers").is_none(),
        "provider state belongs to profile rules and plugin/runtime status, not settings"
    );
    assert!(
        serialized.get("policy").is_none(),
        "retired policy maps must stay out of settings response"
    );
}
