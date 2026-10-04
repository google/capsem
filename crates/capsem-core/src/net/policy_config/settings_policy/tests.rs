use super::*;
use crate::net::policy_config::{load_settings_file, DetectionLevel, SecurityPluginMode};

struct Home {
    dir: tempfile::TempDir,
}

impl Home {
    fn new(settings: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        if !settings.is_empty() {
            std::fs::write(dir.path().join("settings.toml"), settings).unwrap();
        }
        Self { dir }
    }

    fn path(&self) -> PathBuf {
        self.dir.path().join("settings.toml")
    }

    fn edit(&self) -> SettingsPolicyEdit {
        SettingsPolicyEdit::open(&self.path()).expect("settings open for edit")
    }

    fn loaded(&self) -> SettingsFile {
        load_settings_file(&self.path()).expect("settings load")
    }

    fn text(&self) -> String {
        std::fs::read_to_string(self.path()).unwrap()
    }
}

fn corp(input: &str) -> SettingsFile {
    toml::from_str(input).expect("corp TOML parses")
}

fn rewrite_informational() -> SecurityPluginConfig {
    SecurityPluginConfig {
        mode: SecurityPluginMode::Disable,
        detection_level: DetectionLevel::High,
    }
}

#[test]
fn a_plugin_edit_writes_settings_and_reports_the_change() {
    let home = Home::new("");
    let summary = home
        .edit()
        .set_plugin_config(
            &SettingsFile::default(),
            "credential_broker",
            rewrite_informational(),
            "test",
        )
        .expect("plugin edit applies");

    assert_eq!(home.loaded().plugins["credential_broker"], rewrite_informational());
    assert_eq!(summary.filename, "settings.toml");
    assert_eq!(summary.affected_path, "settings.toml");
    assert_eq!(summary.category, "plugin");
    assert_eq!(summary.target_kind, "plugin");
    assert_eq!(summary.target_key, "credential_broker");
    assert_eq!(summary.operation, "edit");
    assert_eq!(summary.actor, "test");
    assert_eq!(summary.rule_id, None);
    assert_eq!(summary.old_size, 0);
    assert_eq!(summary.new_size, home.text().len() as u64);
    assert_ne!(summary.old_hash, summary.new_hash);
    assert!(summary.new_hash.starts_with("blake3:") && summary.new_hash.len() == 71);
}

#[test]
fn a_plugin_corp_decides_is_refused() {
    let home = Home::new("");
    let corp = corp("[plugins.credential_broker]\nmode = \"rewrite\"\n");
    let error = home
        .edit()
        .set_plugin_config(&corp, "credential_broker", rewrite_informational(), "test")
        .expect_err("corp owns the plugin");
    assert!(error.contains("corp config"), "{error}");
    assert!(!home.path().exists(), "a refused edit writes nothing");
}

#[test]
fn the_mcp_default_starts_from_the_builtin_rule_and_reads_back_from_settings() {
    let home = Home::new("");
    let before = mcp_default_permission(&home.loaded(), &SettingsFile::default()).unwrap();
    assert_eq!(before.source, "default");
    assert_eq!(before.action, SecurityRuleAction::Allow);

    let summary = home
        .edit()
        .set_mcp_default_permission(&SettingsFile::default(), SecurityRuleAction::Ask, "test")
        .expect("default edit applies");
    assert_eq!(summary.rule_id.as_deref(), Some("default.mcp"));
    assert_eq!(summary.target_kind, "mcp_default");

    let after = mcp_default_permission(&home.loaded(), &SettingsFile::default()).unwrap();
    assert_eq!(after.action, SecurityRuleAction::Ask);
    assert_eq!(after.source, "settings");
    let builtin = builtin_default_mcp_rule().unwrap();
    assert_eq!(home.loaded().default["mcp"].condition, builtin.condition);
}

#[test]
fn a_corp_default_mcp_wins_and_refuses_the_edit() {
    let home = Home::new("");
    let corp = corp(
        r#"
[default.mcp]
name = "mcp"
action = "block"
priority = "default"
match = 'has(mcp.method)'
"#,
    );
    assert_eq!(
        mcp_default_permission(&home.loaded(), &corp).unwrap(),
        McpToolPermissionStatus {
            action: SecurityRuleAction::Block,
            source: "corp".to_string(),
            rule_id: Some("default.mcp".to_string()),
        }
    );
    let error = home
        .edit()
        .set_mcp_default_permission(&corp, SecurityRuleAction::Allow, "test")
        .expect_err("corp owns default.mcp");
    assert!(error.contains("corp config"), "{error}");
}

#[test]
fn a_tool_permission_is_one_managed_rule_updated_in_place() {
    let home = Home::new("");
    let none = SettingsFile::default();
    let first = home
        .edit()
        .set_mcp_tool_permission(&none, "local", "fetch_http", SecurityRuleAction::Block, "test")
        .expect("tool edit applies");
    assert_eq!(
        first.rule_id.as_deref(),
        Some("profiles.rules.mcp_local_fetch_http_permission")
    );
    assert_eq!(first.target_kind, "mcp_tool");
    assert_eq!(first.target_key, "local/fetch_http");

    home.edit()
        .set_mcp_tool_permission(&none, "local", "fetch_http", SecurityRuleAction::Ask, "test")
        .expect("second tool edit applies");
    let loaded = home.loaded();
    assert_eq!(loaded.profiles.rules.len(), 1, "{:?}", loaded.profiles.rules.keys());
    assert_eq!(
        mcp_tool_permission(&loaded, &none, "local", "fetch_http").unwrap(),
        McpToolPermissionStatus {
            action: SecurityRuleAction::Ask,
            source: "settings".to_string(),
            rule_id: Some("profiles.rules.mcp_local_fetch_http_permission".to_string()),
        }
    );
    assert_eq!(
        mcp_tool_permission(&loaded, &none, "local", "other_tool")
            .unwrap()
            .source,
        "default"
    );
}

#[test]
fn a_tool_permission_needs_a_configured_server() {
    let home = Home::new("");
    let error = home
        .edit()
        .set_mcp_tool_permission(
            &SettingsFile::default(),
            "nowhere",
            "tool",
            SecurityRuleAction::Block,
            "test",
        )
        .expect_err("undeclared server refused");
    assert!(error.contains("not configured"), "{error}");

    let corp = corp("[mcp.server_enabled]\nlocal = false\n");
    let error = home
        .edit()
        .set_mcp_tool_permission(&corp, "local", "fetch_http", SecurityRuleAction::Block, "test")
        .expect_err("disabled local server refused");
    assert!(error.contains("not configured"), "{error}");

    let declared = Home::new("[[mcp.servers]]\nname = \"wiki\"\nurl = \"https://wiki.example.invalid/mcp\"\n");
    declared
        .edit()
        .set_mcp_tool_permission(
            &SettingsFile::default(),
            "wiki",
            "search",
            SecurityRuleAction::Allow,
            "test",
        )
        .expect("declared server accepted");
}

#[test]
fn only_allow_ask_and_block_are_permissions() {
    let home = Home::new("");
    let error = home
        .edit()
        .set_mcp_default_permission(&SettingsFile::default(), SecurityRuleAction::Rewrite, "test")
        .expect_err("rewrite is not a permission");
    assert!(error.contains("allow, ask, or block"), "{error}");
}

#[test]
fn an_edit_keeps_referenced_rule_files_referenced() {
    let home = Home::new("[rule_files]\nenforcement = \"enforcement.toml\"\n");
    std::fs::write(
        home.dir.path().join("enforcement.toml"),
        r#"
[profiles.rules.block_example]
name = "block_example"
action = "block"
match = 'http.host == "example.invalid"'
"#,
    )
    .unwrap();

    home.edit()
        .set_plugin_config(
            &SettingsFile::default(),
            "credential_broker",
            rewrite_informational(),
            "test",
        )
        .expect("edit applies");
    assert!(!home.text().contains("block_example"), "{}", home.text());
    home.edit()
        .set_plugin_config(
            &SettingsFile::default(),
            "log_sanitizer",
            rewrite_informational(),
            "test",
        )
        .expect("a second edit still loads the file it wrote");
    assert!(home.loaded().profiles.rules.contains_key("block_example"));
}

#[test]
fn a_settings_file_that_does_not_validate_is_not_written() {
    let original = "[network.dns]\nupstreams = [\"127.0.0.1:53\"]\n";
    let home = Home::new(original);
    let error = home
        .edit()
        .set_plugin_config(
            &SettingsFile::default(),
            "credential_broker",
            rewrite_informational(),
            "test",
        )
        .expect_err("network mechanics are corp-only");
    assert!(error.contains("network"), "{error}");
    assert_eq!(home.text(), original);
}
