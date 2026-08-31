use super::*;
use crate::{DetectionLevel, SecurityRuleAction};

const DRAFT: &str = include_str!("../default_provider_rules.toml");

#[test]
fn parses_real_provider_defaults_as_security_rules() {
    let profile = ProviderRuleProfile::parse_toml(DRAFT).expect("draft parses");
    assert_eq!(
        profile.ai.keys().cloned().collect::<Vec<_>>(),
        vec!["anthropic", "google", "ollama", "openai"]
    );
    let compiled = profile
        .compile(SecurityRuleSource::BuiltinDefault)
        .expect("draft compiles");
    assert!(compiled
        .iter()
        .any(|rule| rule.rule_id == "profiles.rules.ai_openai_http_api"));
    let built_in_defaults = ProviderRuleProfile::builtin_security_defaults();
    let built_in_compiled = built_in_defaults
        .compile(SecurityRuleSource::BuiltinDefault)
        .expect("full built-in defaults compile");
    let unknown_provider_rule = built_in_compiled
        .iter()
        .find(|rule| rule.rule_id == "profiles.rules.default_unknown_model_provider")
        .expect("built-in defaults include unknown provider detection");
    assert_eq!(unknown_provider_rule.action, SecurityRuleAction::Allow);
    assert_eq!(
        unknown_provider_rule.detection_level,
        Some(DetectionLevel::Informational)
    );
    assert_eq!(unknown_provider_rule.condition, r#"model.provider == "unknown""#);
    let unknown_mcp_rule = built_in_compiled
        .iter()
        .find(|rule| rule.rule_id == "profiles.rules.default_unknown_mcp_server")
        .expect("built-in defaults include unknown MCP detection");
    assert_eq!(unknown_mcp_rule.action, SecurityRuleAction::Allow);
    assert_eq!(unknown_mcp_rule.detection_level, Some(DetectionLevel::Informational));
    assert_eq!(unknown_mcp_rule.condition, r#"mcp.server.name.contains("observed:")"#);
    assert!(built_in_defaults.plugins.contains_key("credential_broker"));
    assert!(built_in_defaults.plugins.contains_key("log_sanitizer"));
    assert!(compiled.iter().all(|rule| !rule.condition.contains("file.ingress")));
    assert!(compiled.iter().all(|rule| !rule.condition.contains("credential.name")));
}

#[test]
fn builtin_profile_contract_requires_plugins_and_visible_default_rules() {
    let missing_plugins = SecurityRuleProfile::parse_toml(
        r#"
    [default.http]
    name = "http"
    action = "allow"
    priority = "default"
    reason = "Default allow for HTTP requests."
match = 'has(http.host)'
"#,
    )
    .expect("profile without plugins parses before built-in contract");
    let err =
        validate_builtin_profile_contract(&missing_plugins).expect_err("built-in profile requires plugin section");
    assert!(err.contains("[plugins.credential_broker]"), "{err}");

    let missing_defaults = SecurityRuleProfile::parse_toml(
        r#"
[plugins.credential_broker]
mode = "rewrite"

[plugins.log_sanitizer]
mode = "rewrite"
"#,
    )
    .expect("profile without defaults parses before built-in contract");
    let err =
        validate_builtin_profile_contract(&missing_defaults).expect_err("built-in profile requires visible defaults");
    assert!(err.contains("[default.http]"), "{err}");
}

#[test]
fn provider_defaults_build_settings_defined_endpoint_registry() {
    let registry = ProviderRuleProfile::builtin_defaults()
        .endpoint_registry()
        .expect("registry builds");
    assert_eq!(registry.len(), 4);
    assert_eq!(registry.get("openai").expect("openai").protocol, ModelProtocol::OpenAi);
    assert_eq!(
        registry.get("anthropic").expect("anthropic").protocol,
        ModelProtocol::Anthropic
    );
    assert_eq!(registry.get("google").expect("google").protocol, ModelProtocol::Google);
    assert_eq!(registry.get("ollama").expect("ollama").protocol, ModelProtocol::Ollama);
    assert_eq!(
        registry.protocol_for_host("api.openai.com"),
        Some(ModelProtocol::OpenAi)
    );
    assert_eq!(
        registry.protocol_for_host("GENERATIVELANGUAGE.GOOGLEAPIS.COM."),
        Some(ModelProtocol::Google)
    );
    assert_eq!(
        registry.protocol_for_host("daily-cloudcode-pa.googleapis.com"),
        Some(ModelProtocol::Google)
    );
    assert_eq!(registry.protocol_for_host("127.0.0.1"), Some(ModelProtocol::Ollama));
    assert_eq!(registry.protocol_for_host("local.ollama"), Some(ModelProtocol::Ollama));
    assert_eq!(
        registry.protocol_for_target("local.ollama", 11434),
        Some(ModelProtocol::Ollama)
    );
    assert_eq!(registry.protocol_for_target("local.ollama", 80), None);
    assert_eq!(
        registry.protocol_for_target("api.openai.com", 443),
        Some(ModelProtocol::OpenAi)
    );
    assert_eq!(registry.protocol_for_target("api.openai.com", 80), None);
    let openai = registry.get("openai").expect("openai endpoint");
    assert_eq!(openai.listen_ports, vec![443]);
    assert_eq!(openai.allowed_remote_targets, vec!["api.openai.com:443"]);
}

#[test]
fn custom_openai_compatible_endpoint_schema_requires_no_protocol_enum_growth() {
    let profile = ProviderRuleProfile::parse_toml(
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
    .expect("profile parses");

    let registry = profile.endpoint_registry().expect("registry builds");
    let endpoint = registry.get("private_gateway").expect("private endpoint exists");
    assert_eq!(endpoint.provider_id, "private_gateway");
    assert_eq!(endpoint.display_name, "Private Gateway");
    assert_eq!(endpoint.protocol, ModelProtocol::OpenAi);
    assert_eq!(endpoint.upstream_url, "https://llm.internal.example/v1");
    assert_eq!(
        registry.protocol_for_host("llm.internal.example"),
        Some(ModelProtocol::OpenAi)
    );
    assert_eq!(
        registry.protocol_for_host("company-openai"),
        Some(ModelProtocol::OpenAi)
    );
    assert_eq!(
        registry.protocol_for_target("company-openai", 8443),
        Some(ModelProtocol::OpenAi)
    );
    assert_eq!(registry.protocol_for_target("company-openai", 11434), None);
}

#[test]
fn provider_endpoint_aliases_are_rejected_in_favor_of_explicit_targets() {
    let error = ProviderRuleProfile::parse_toml(
        r#"
[ai.private_gateway]
name = "Private Gateway"
protocol = "openai-compatible"
url = "https://llm.internal.example/v1"
aliases = ["company-openai"]
allowed_remote_targets = ["company-openai:443"]

[ai.private_gateway.rules.http_api]
name = "private_gateway_http_seen"
action = "allow"
match = 'http.host == "company-openai"'
"#,
    )
    .expect_err("provider aliases are a second classifier and must be rejected");
    assert!(error.contains("aliases"), "{error}");
    assert!(error.contains("unknown field"), "{error}");
}

#[test]
fn provider_endpoint_metadata_rejects_static_credentials_and_config_files() {
    for (field, value) in [
        ("credential_setting_id", r#""ai.private_gateway.api_key""#),
        (
            "credential_ref",
            r#""credential:blake3:2222222222222222222222222222222222222222222222222222222222222222""#,
        ),
        ("files", r#"["/root/.config/private-gateway/config.toml"]"#),
    ] {
        let input = format!(
            r#"
[ai.private_gateway]
name = "Private Gateway"
protocol = "openai-compatible"
url = "https://llm.internal.example/v1"
{field} = {value}

[ai.private_gateway.rules.http_api]
name = "private_gateway_http_seen"
action = "allow"
match = 'http.host == "llm.internal.example"'
"#
        );
        let err = ProviderRuleProfile::parse_toml(&input)
            .expect_err("provider static credential/config metadata must be rejected");
        assert!(err.contains(field), "{field}: {err}");
    }
}

#[test]
fn provider_override_uses_same_rule_contract() {
    let user = ProviderRuleProfile::parse_toml(
        r#"
[ai.openai]
name = "OpenAI"
protocol = "openai"
url = "https://api.openai.com/v1"

[ai.openai.rules.http_api]
name = "openai_http_user"
action = "ask"
match = 'http.host == "api.openai.com"'
"#,
    )
    .expect("user provider parses");
    let corp = ProviderRuleProfile::parse_toml(
        r#"
[ai.openai]
name = "OpenAI"
protocol = "openai"
url = "https://api.openai.com/v1"

[ai.openai.rules.http_api]
name = "openai_http_corp_block"
action = "block"
detection_level = "critical"
priority = -100
match = 'http.host == "api.openai.com"'
"#,
    )
    .expect("corp provider parses");

    let merged = ProviderRuleProfile::merge_override(&user, &corp).expect("merge succeeds");
    let compiled = merged
        .compile(SecurityRuleSource::Corp)
        .expect("merged profile compiles");
    let rule = compiled
        .iter()
        .find(|rule| rule.rule_id == "profiles.rules.ai_openai_http_api")
        .expect("merged rule exists");
    assert_eq!(rule.name, "openai_http_corp_block");
    assert_eq!(rule.action, SecurityRuleAction::Block);
    assert_eq!(rule.detection_level, Some(DetectionLevel::Critical));
    assert_eq!(rule.priority, -100);
}

#[test]
fn provider_owned_rules_compile_to_security_event_rule_contract() {
    let profile = ProviderRuleProfile::parse_toml(
        r#"
[ai.openai]
name = "OpenAI"
protocol = "openai"
url = "https://api.openai.com/v1"

[ai.openai.rules.detect_http]
name = "openai_detect_http"
action = "allow"
detection_level = "informational"
match = 'http.host.matches("(^|.*\.)openai\.com$")'

"#,
    )
    .expect("provider rules parse");

    let rules = profile
        .compile_rule_set(SecurityRuleSource::User)
        .expect("provider rules compile");
    let ids = rules
        .rules()
        .iter()
        .map(|rule| (rule.rule_id.as_str(), rule.action, rule.detection_level, rule.priority))
        .collect::<Vec<_>>();

    assert!(ids.contains(&(
        "profiles.rules.ai_openai_detect_http",
        SecurityRuleAction::Allow,
        Some(DetectionLevel::Informational),
        10,
    )));
}
