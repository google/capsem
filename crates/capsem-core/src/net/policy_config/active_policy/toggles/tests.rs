use super::*;
use capsem_config::{SettingEntry, SettingValue};

fn file(entries: &[(&str, SettingValue)]) -> SettingsFile {
    let mut file = SettingsFile::default();
    for (id, value) in entries {
        file.settings.insert(
            (*id).to_string(),
            SettingEntry {
                value: value.clone(),
                modified: "2026-01-01T00:00:00Z".into(),
            },
        );
    }
    file
}

const NPM: &str = "security.services.registry.npm.allow";

#[test]
fn an_enabled_toggle_adds_nothing_because_http_is_allowed_by_default() {
    let (user, corp) = toggle_block_rules(&SettingsFile::default(), &SettingsFile::default()).unwrap();
    assert!(user.is_empty() && corp.is_empty(), "{user:?} {corp:?}");
}

#[test]
fn a_toggle_the_user_turns_off_blocks_its_domains_ahead_of_the_default() {
    let (user, corp) =
        toggle_block_rules(&file(&[(NPM, SettingValue::Bool(false))]), &SettingsFile::default()).unwrap();
    assert!(corp.is_empty());
    let rule = user.get("toggle_security_services_registry_npm").expect("block rule");
    assert_eq!(rule.action, SecurityRuleAction::Block);
    assert_eq!(
        rule.priority,
        Some(SecurityRulePriority::Explicit(capsem_config::USER_PRIORITY_MAX))
    );
    assert!(
        rule.condition.contains(r#"http.host == "registry.npmjs.org""#),
        "{}",
        rule.condition
    );
    assert!(
        rule.condition.contains(r#"http.host.endsWith(".npmjs.org")"#),
        "{}",
        rule.condition
    );
}

#[test]
fn corp_turning_a_toggle_off_is_a_corp_rule_the_user_cannot_undo() {
    let user_on = file(&[(NPM, SettingValue::Bool(true))]);
    let corp_off = file(&[(NPM, SettingValue::Bool(false))]);
    let (user, corp) = toggle_block_rules(&user_on, &corp_off).unwrap();
    assert!(user.is_empty());
    let rule = corp
        .get("toggle_security_services_registry_npm")
        .expect("corp block rule");
    assert_eq!(
        rule.priority,
        Some(SecurityRulePriority::Explicit(capsem_config::CORP_PRIORITY_MAX))
    );
}

#[test]
fn the_domains_setting_names_what_a_disabled_toggle_blocks() {
    let settings = file(&[
        (NPM, SettingValue::Bool(false)),
        (
            "security.services.registry.npm.domains",
            SettingValue::Text("npm.example.com, *.mirror.example".into()),
        ),
    ]);
    let (user, _) = toggle_block_rules(&settings, &SettingsFile::default()).unwrap();
    let condition = &user["toggle_security_services_registry_npm"].condition;
    assert!(condition.contains(r#"http.host == "npm.example.com""#), "{condition}");
    assert!(
        condition.contains(r#"http.host.endsWith(".mirror.example")"#),
        "{condition}"
    );
    assert!(!condition.contains("npmjs"), "{condition}");
}

/// The domains setting is free text a user writes; nothing in it can become
/// rule syntax.
#[test]
fn a_domain_pattern_that_is_not_a_hostname_is_refused() {
    for hostile in [r#"x" || true || "y"#, "a b.com", "*.*.com", "evil.com)"] {
        let settings = file(&[
            (NPM, SettingValue::Bool(false)),
            (
                "security.services.registry.npm.domains",
                SettingValue::Text(hostile.into()),
            ),
        ]);
        let error = toggle_block_rules(&settings, &SettingsFile::default()).unwrap_err();
        assert!(
            error.contains("security.services.registry.npm.domains"),
            "{hostile}: {error}"
        );
    }
}

/// GitLab's toggle defaults to off, yet HTTP has always reached gitlab.com:
/// a default nobody chose never starts blocking.
#[test]
fn a_toggle_left_at_an_off_default_blocks_nothing() {
    let (user, corp) = toggle_block_rules(&SettingsFile::default(), &SettingsFile::default()).unwrap();
    assert!(!user.contains_key("toggle_repository_providers_gitlab"));
    assert!(corp.is_empty());
    let off = file(&[("repository.providers.gitlab.allow", SettingValue::Bool(false))]);
    let (user, _) = toggle_block_rules(&off, &SettingsFile::default()).unwrap();
    assert!(user["toggle_repository_providers_gitlab"]
        .condition
        .contains(r#"http.host == "gitlab.com""#));
}
