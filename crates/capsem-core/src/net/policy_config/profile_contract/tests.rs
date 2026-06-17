use super::*;

fn parse_profile(input: &str) -> ProfileConfigFile {
    toml::from_str(input).expect("profile TOML parses")
}

const MINIMAL_ASSETS: &str = r#"
[assets]
format = "profile-assets.v1"
refresh_policy = "24h"

[assets.arch.arm64.kernel]
name = "vmlinuz"
url = "file:///tmp/vmlinuz"
hash = "blake3:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
size = 1

[assets.arch.arm64.initrd]
name = "initrd.img"
url = "file:///tmp/initrd.img"
hash = "blake3:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
size = 1

[assets.arch.arm64.rootfs]
name = "rootfs.erofs"
url = "file:///tmp/rootfs.erofs"
hash = "blake3:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
size = 1
"#;

#[test]
fn profile_config_requires_assets_section() {
    let error = toml::from_str::<ProfileConfigFile>(
        r#"
id = "developer"
name = "Developer"
description = "Developer profile"
revision = "2026.06.14.1"
refresh_policy = "24h"
"#,
    )
    .expect_err("profile assets must be explicit");

    assert!(
        error.to_string().contains("missing field `assets`"),
        "unexpected parse error: {error}"
    );
}

#[test]
fn profile_config_file_owns_full_profile_behavior_contract() {
    let profile = parse_profile(
        r#"
id = "developer"
name = "Developer"
description = "Default developer VM profile."
icon_svg = "<svg viewBox=\"0 0 16 16\"></svg>"
revision = "2026.0607.1"
refresh_policy = "24h"

[availability]
web = true
shell = true
mobile = false

[assets]
format = "profile-assets.v1"
refresh_policy = "on_profile_refresh"

[assets.arch.arm64.kernel]
name = "vmlinuz"
url = "https://example.invalid/arm64-vmlinuz"
hash = "blake3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
size = 1

[assets.arch.arm64.initrd]
name = "initrd.img"
url = "https://example.invalid/arm64-initrd.img"
hash = "blake3:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
size = 1

[assets.arch.arm64.rootfs]
name = "rootfs.erofs"
url = "https://example.invalid/arm64-rootfs.erofs"
hash = "blake3:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
size = 1

[obom]
format = "cyclonedx-obom.v1"

[obom.arch.arm64]
name = "obom.cdx.json"
url = "https://example.invalid/arm64-obom.cdx.json"
hash = "blake3:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
size = 1
generator = "cdxgen"
generator_version = "11.0.0"

[vm]
cpu_count = 6
ram_gb = 8
scratch_disk_size_gb = 32

[rule_files]
enforcement = "rules/enforcement.toml"
sigma = "rules/detection.yaml"

[files.mcp]
path = "profiles/developer/mcp.json"
hash = "blake3:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"
size = 1

[files.apt_packages]
path = "profiles/developer/apt-packages.txt"
hash = "blake3:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
size = 1

[files.root_manifest]
path = "profiles/developer/root.manifest.json"
hash = "blake3:1111111111111111111111111111111111111111111111111111111111111111"
size = 1

[files.build]
path = "profiles/developer/build.sh"
hash = "blake3:2222222222222222222222222222222222222222222222222222222222222222"
size = 1

[default.http]
name = "default_http"
action = "allow"
priority = "default"
reason = "Default allow for HTTP requests."
match = 'has(http.host)'

[profiles.rules.skill_loaded]
name = "skill_loaded"
action = "allow"
detection_level = "informational"
match = 'file.read.path.contains("skills/")'

[ai.openai]
name = "OpenAI"
protocol = "openai"
url = "https://api.openai.com/v1"
listen_ports = [443]
allowed_remote_targets = ["api.openai.com:443"]

[ai.openai.rules.http_api]
name = "openai_http_api"
action = "allow"
match = 'http.host == "api.openai.com"'

[plugins.dummy_pre_eicar]
mode = "block"
detection_level = "critical"

[mcp]
health_check_interval_secs = 60

[mcp.server_enabled]
local = true

[skills]
paths = ["/root/.codex/skills/security/SKILL.md"]

"#,
    );

    profile.validate().expect("profile contract validates");
    assert_eq!(profile.id, "developer");
    assert_eq!(profile.assets.arch["arm64"].rootfs.name, "rootfs.erofs");
    assert_eq!(
        profile.obom.as_ref().unwrap().arch["arm64"].generator,
        "cdxgen"
    );
    assert_eq!(profile.vm.cpu_count, 6);
    assert_eq!(
        profile.rule_files.enforcement.as_deref(),
        Some("rules/enforcement.toml")
    );
    assert_eq!(
        profile.rule_files.sigma.as_deref(),
        Some("rules/detection.yaml")
    );
    assert_eq!(
        profile
            .files
            .mcp
            .as_ref()
            .map(|descriptor| descriptor.path.as_str()),
        Some("profiles/developer/mcp.json")
    );
    assert_eq!(
        profile
            .files
            .build
            .as_ref()
            .map(|descriptor| descriptor.path.as_str()),
        Some("profiles/developer/build.sh")
    );
    assert!(profile.default.contains_key("http"));
    assert!(profile.profiles.rules.contains_key("skill_loaded"));
    assert!(profile.ai.contains_key("openai"));
    assert!(profile.plugins.contains_key("dummy_pre_eicar"));
    assert_eq!(
        profile.mcp.unwrap().server_enabled.get("local").copied(),
        Some(true)
    );
}

#[test]
fn profile_config_rejects_stale_install_file_reference() {
    let error = toml::from_str::<ProfileConfigFile>(
        r#"
id = "developer"
name = "Developer"
description = "Developer profile"
revision = "2026.06.12.1"
refresh_policy = "24h"

[files.install]
path = "profiles/developer/install.sh"
hash = "blake3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
size = 1
"#,
    )
    .expect_err("files.install is not a supported profile contract");

    assert!(
        error.to_string().contains("unknown field `install`"),
        "unexpected parse error: {error}"
    );
}

#[test]
fn profile_file_refs_reject_unpinned_or_escape_paths() {
    let base = format!(
        r#"
id = "developer"
name = "Developer"
description = "Developer profile"
revision = "2026.06.09.1"
refresh_policy = "24h"
{MINIMAL_ASSETS}

[files.mcp]
path = "profiles/developer/mcp.json"
hash = "blake3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
size = 1
"#
    );
    parse_profile(&base)
        .validate()
        .expect("valid profile file ref");

    let absolute = base.replace(
        "path = \"profiles/developer/mcp.json\"",
        "path = \"/etc/passwd\"",
    );
    assert!(parse_profile(&absolute)
        .validate()
        .unwrap_err()
        .contains("config-root-relative"));

    let traversal = base.replace(
        "path = \"profiles/developer/mcp.json\"",
        "path = \"profiles/developer/../corp.toml\"",
    );
    assert!(parse_profile(&traversal)
        .validate()
        .unwrap_err()
        .contains("path traversal"));

    let bad_hash = base.replace(
        "blake3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    );
    assert!(parse_profile(&bad_hash)
        .validate()
        .unwrap_err()
        .contains("blake3"));

    let zero_size = base.replace("size = 1", "size = 0");
    assert!(parse_profile(&zero_size)
        .validate()
        .unwrap_err()
        .contains("size"));
}

#[test]
fn profile_config_rejects_static_tool_config_sources() {
    let error = toml::from_str::<ProfileConfigFile>(
        r#"
id = "developer"
name = "Developer"
description = "Developer profile"
revision = "2026.06.08.3"
refresh_policy = "24h"

[tool_config_sources.codex]
tool_id = "codex"
guest_path = "/root/.codex/config.toml"
format = "toml"
"#,
    )
    .expect_err("tool_config_sources are runtime ledger evidence, not static profile config");

    assert!(error.to_string().contains("tool_config_sources"), "{error}");
}

#[test]
fn builtin_primary_profile_manifest_is_valid_and_erofs_backed() {
    let profile = ProfileConfigFile::builtin_primary();

    profile
        .validate()
        .expect("builtin primary profile validates");
    assert_eq!(profile.id, "code");
    assert_eq!(profile.name, "Code");
    assert_eq!(
        profile
            .assets
            .current_arch_assets()
            .expect("current architecture assets")
            .rootfs
            .name,
        "rootfs.erofs"
    );
    assert!(profile.availability.web);
    assert!(profile.availability.shell);
    assert_eq!(
        profile.rule_files.enforcement.as_deref(),
        Some("profiles/code/enforcement.toml")
    );
    assert_eq!(
        profile.rule_files.sigma.as_deref(),
        Some("profiles/code/detection.yaml")
    );
    assert!(profile.plugins.contains_key("credential_broker"));
}

#[test]
fn profile_config_rejects_credential_broker_settings() {
    let error = toml::from_str::<ProfileConfigFile>(
        r#"
id = "developer"
name = "Developer"
description = "Default developer VM profile."
revision = "2026.0607.1"
refresh_policy = "24h"

[credentials]
broker_enabled = true
"#,
    )
    .expect_err("credential broker config is plugin-owned, not a profile credential block");
    assert!(error.to_string().contains("unknown field `credentials`"));
}

#[test]
fn profile_config_rejects_ui_settings_soup() {
    let error = toml::from_str::<ProfileConfigFile>(
        r#"
id = "developer"
name = "Developer"
description = "Default developer VM profile."
revision = "2026.0607.1"
refresh_policy = "24h"

[settings."appearance.dark_mode"]
value = true
modified = "2026-06-07T00:00:00Z"
"#,
    )
    .expect_err("profile files must not accept settings.toml sections");
    assert!(error.to_string().contains("unknown field `settings`"));
}

#[test]
fn profile_config_validation_rejects_bad_identity_assets_and_vm_defaults() {
    let mut profile = ProfileConfigFile::builtin_primary();
    profile.id = "Bad Profile".to_string();
    assert!(profile.validate().unwrap_err().contains("lowercase ascii"));

    profile.id = "developer".to_string();
    profile.icon_svg = Some("<div></div>".to_string());
    assert!(profile.validate().unwrap_err().contains("icon_svg"));

    profile.icon_svg = Some("<svg></svg>".to_string());
    profile.vm.cpu_count = 0;
    assert!(profile.validate().unwrap_err().contains("cpu_count"));

    profile.vm.cpu_count = 4;
    profile.assets.arch.clear();
    assert!(profile.validate().unwrap_err().contains("assets.arch"));
}

#[test]
fn checked_in_code_profile_parses_and_validates() {
    let profile = toml::from_str::<ProfileConfigFile>(include_str!(
        "../../../../../../config/profiles/code/profile.toml"
    ))
    .expect("checked-in code profile parses");

    profile
        .validate()
        .expect("checked-in code profile validates");
    assert_eq!(profile.id, "code");
    assert!(profile.assets.arch.contains_key("arm64"));
    assert!(profile.assets.arch.contains_key("x86_64"));
    assert!(profile.plugins.contains_key("credential_broker"));
    assert_eq!(
        profile
            .files
            .enforcement
            .as_ref()
            .map(|descriptor| descriptor.path.as_str()),
        Some("profiles/code/enforcement.toml")
    );
    assert_eq!(
        profile
            .files
            .detection
            .as_ref()
            .map(|descriptor| descriptor.path.as_str()),
        Some("profiles/code/detection.yaml")
    );
    assert_eq!(
        profile
            .mcp
            .as_ref()
            .and_then(|mcp| mcp.server_enabled.get("local"))
            .copied(),
        Some(true)
    );
}

#[test]
fn profile_check_rejects_mutated_pinned_rule_file() {
    let fixture = ProfileFixture::new();
    let profile = Profile::load_from_dir(fixture.profile_dir()).expect("profile loads");
    profile
        .check(&fixture.assets_dir(), "arm64")
        .expect("fixture is initially ready");

    std::fs::write(
        fixture.config_root().join("profiles/code/enforcement.toml"),
        "[default.http]\nname = \"http\"\naction = \"allow\"\npriority = \"default\"\nmatch = 'has(http.host)'\n",
    )
    .unwrap();

    let error = profile
        .check(&fixture.assets_dir(), "arm64")
        .expect_err("tampered enforcement file fails profile check");
    assert!(error.contains("enforcement"), "{error}");
}

#[test]
fn profile_download_assets_uses_file_url_same_status_path() {
    let fixture = ProfileFixture::new_without_downloaded_assets();
    let profile = Profile::load_from_dir(fixture.profile_dir()).expect("profile loads");
    assert!(!profile.status(&fixture.assets_dir(), "arm64").ready);

    let status = profile
        .download_assets(&fixture.assets_dir(), "arm64")
        .expect("file URL assets download through profile rail");

    assert!(status.ready, "{status:?}");
    assert_eq!(status.assets.len(), 3);
    assert!(status
        .assets
        .iter()
        .all(|asset| asset.present && asset.valid));
}

#[test]
fn active_profile_materializes_corp_network_mechanics() {
    let fixture = ProfileFixture::new();
    let profile = Profile::load_from_dir(fixture.profile_dir()).expect("profile loads");
    let corp: SettingsFile = toml::from_str(
        r#"
refresh_policy = "24h"

[settings."vm.resources.log_bodies"]
value = true
modified = "2026-06-14T00:00:00Z"

[settings."vm.resources.max_body_capture"]
value = 8192
modified = "2026-06-14T00:00:00Z"

[settings."security.web.http_upstream_ports"]
value = [80, 3713, 8080]
modified = "2026-06-14T00:00:00Z"

[network.dns]
upstreams = ["127.0.0.1:5353"]
"#,
    )
    .expect("corp TOML parses");

    let active = ActiveProfileFile::from_profile_and_corp(&profile, &corp, BTreeMap::new())
        .expect("active profile materializes");

    assert_eq!(active.network.log_bodies, Some(true));
    assert_eq!(active.network.max_body_capture, Some(8192));
    assert_eq!(active.network.http_upstream_ports, vec![80, 3713, 8080]);
    assert_eq!(
        active.network.dns.upstreams,
        vec!["127.0.0.1:5353".to_string()]
    );
}

#[test]
fn profile_mcp_tool_permission_mutation_updates_rule_and_pin() {
    let fixture = ProfileFixture::new();
    let mut profile = Profile::load_from_dir(fixture.profile_dir()).expect("profile loads");
    let initial = profile
        .mcp_tool_permission("capsem", "fetch_http")
        .expect("default MCP permission resolves");
    assert_eq!(initial.action, SecurityRuleAction::Allow);
    assert_eq!(initial.source, "default");
    assert_eq!(initial.rule_id.as_deref(), Some("default.mcp"));

    let old_pin = profile
        .config()
        .files
        .enforcement
        .as_ref()
        .unwrap()
        .hash
        .clone();

    let summary = profile
        .set_mcp_tool_permission("capsem", "fetch_http", SecurityRuleAction::Ask, "ui")
        .expect("MCP tool permission mutation succeeds");

    assert_eq!(summary.profile_id, "code");
    assert_eq!(summary.category, "mcp");
    assert_eq!(summary.filename, "enforcement.toml");
    assert_eq!(summary.target_kind, "mcp_tool");
    assert_eq!(summary.target_key, "capsem/fetch_http");
    assert_eq!(
        summary.rule_id.as_deref(),
        Some("profiles.rules.mcp_capsem_fetch_http_permission")
    );
    assert_ne!(Some(summary.new_hash.clone()), old_pin);

    let reloaded = Profile::load_from_dir(fixture.profile_dir()).expect("profile reloads");
    let permission = reloaded
        .mcp_tool_permission("capsem", "fetch_http")
        .expect("managed MCP permission resolves");
    assert_eq!(permission.action, SecurityRuleAction::Ask);
    assert_eq!(permission.source, "profile_managed");
    assert_eq!(
        permission.rule_id.as_deref(),
        Some("profiles.rules.mcp_capsem_fetch_http_permission")
    );

    let new_pin = reloaded
        .config()
        .files
        .enforcement
        .as_ref()
        .unwrap()
        .hash
        .clone();
    assert_eq!(new_pin, Some(summary.new_hash));
    reloaded
        .check(&fixture.assets_dir(), "arm64")
        .expect("mutation keeps profile ledger valid");

    let rules = reloaded
        .config()
        .security_rule_profile_from_files(reloaded.config_root())
        .expect("mutated rules compile from files");
    let rule = rules
        .profiles
        .rules
        .get("mcp_capsem_fetch_http_permission")
        .expect("managed permission rule exists");
    assert_eq!(rule.action, SecurityRuleAction::Ask);
    assert_eq!(
        rule.managed,
        Some(SecurityRuleManagedTarget::McpTool {
            server: "capsem".to_string(),
            tool: "fetch_http".to_string(),
            operation: SecurityRuleManagedOperation::Permission,
        })
    );
}

#[test]
fn profile_mcp_default_permission_mutation_updates_rule_pin_and_default_tool_permission() {
    let fixture = ProfileFixture::new();
    let mut profile = Profile::load_from_dir(fixture.profile_dir()).expect("profile loads");
    let initial_default = profile
        .mcp_default_permission()
        .expect("default MCP permission resolves");
    assert_eq!(initial_default.action, SecurityRuleAction::Allow);
    assert_eq!(initial_default.source, "default");
    assert_eq!(initial_default.rule_id.as_deref(), Some("default.mcp"));

    let old_pin = profile
        .config()
        .files
        .enforcement
        .as_ref()
        .unwrap()
        .hash
        .clone();

    let summary = profile
        .set_mcp_default_permission(SecurityRuleAction::Ask, "ui")
        .expect("default MCP permission mutation succeeds");
    assert_eq!(summary.profile_id, "code");
    assert_eq!(summary.category, "mcp");
    assert_eq!(summary.target_kind, "mcp_default");
    assert_eq!(summary.target_key, "default.mcp");
    assert_eq!(summary.rule_id.as_deref(), Some("default.mcp"));
    assert_ne!(Some(summary.new_hash.clone()), old_pin);

    let reloaded = Profile::load_from_dir(fixture.profile_dir()).expect("profile reloads");
    let default = reloaded
        .mcp_default_permission()
        .expect("default MCP permission resolves after mutation");
    assert_eq!(default.action, SecurityRuleAction::Ask);
    assert_eq!(default.source, "default");

    let inherited_default = reloaded
        .mcp_tool_permission("capsem", "fetch_http")
        .expect("tool inherits default permission");
    assert_eq!(inherited_default.action, SecurityRuleAction::Ask);
    assert_eq!(inherited_default.source, "default");

    let new_pin = reloaded
        .config()
        .files
        .enforcement
        .as_ref()
        .unwrap()
        .hash
        .clone();
    assert_eq!(new_pin, Some(summary.new_hash));
    reloaded
        .check(&fixture.assets_dir(), "arm64")
        .expect("default mutation keeps profile ledger valid");
}

#[test]
fn profile_mcp_server_mutation_persists_profile_toml_and_permissions() {
    let fixture = ProfileFixture::new();
    let mut profile = Profile::load_from_dir(fixture.profile_dir()).expect("profile loads");

    let summary = profile
        .upsert_mcp_server(
            crate::mcp::policy::McpManualServer {
                name: "github".to_string(),
                url: "https://mcp.invalid/github".to_string(),
                headers: Default::default(),
                auth: None,
                enabled: true,
            },
            "ui",
        )
        .expect("MCP server mutation succeeds");

    assert_eq!(summary.profile_id, "code");
    assert_eq!(summary.category, "mcp");
    assert_eq!(summary.filename, "profile.toml");
    assert_eq!(summary.affected_path, "profiles/code/profile.toml");
    assert_eq!(summary.target_kind, "mcp_server");
    assert_eq!(summary.target_key, "github");
    assert_eq!(summary.operation, "upsert");
    assert!(summary.rule_id.is_none());

    let reloaded = Profile::load_from_dir(fixture.profile_dir()).expect("profile reloads");
    assert!(reloaded
        .config()
        .mcp
        .as_ref()
        .unwrap()
        .servers
        .iter()
        .any(|server| server.name == "github"
            && server.url == "https://mcp.invalid/github"
            && server.enabled));

    let permission = reloaded
        .mcp_tool_permission("github", "search_repos")
        .expect("profile-owned MCP server is known for tool permissions");
    assert_eq!(permission.action, SecurityRuleAction::Allow);
    assert_eq!(permission.source, "default");

    let mut profile = reloaded;
    let delete = profile
        .delete_mcp_server("github", "ui")
        .expect("MCP server delete mutation succeeds");
    assert_eq!(delete.target_kind, "mcp_server");
    assert_eq!(delete.target_key, "github");
    assert_eq!(delete.operation, "delete");

    let reloaded = Profile::load_from_dir(fixture.profile_dir()).expect("profile reloads");
    assert!(!reloaded
        .config()
        .mcp
        .as_ref()
        .unwrap()
        .servers
        .iter()
        .any(|server| server.name == "github"));
    let error = reloaded
        .mcp_tool_permission("github", "search_repos")
        .expect_err("deleted MCP server is no longer known");
    assert!(
        error.contains("MCP server github is not declared"),
        "{error}"
    );
}

#[test]
fn profile_skill_mutations_persist_profile_toml() {
    let fixture = ProfileFixture::new();
    let mut profile = Profile::load_from_dir(fixture.profile_dir()).expect("profile loads");

    let add = profile
        .add_skill_path("/root/.codex/skills/security/SKILL.md", "ui")
        .expect("skill add mutation succeeds");
    assert_eq!(add.profile_id, "code");
    assert_eq!(add.category, "skills");
    assert_eq!(add.filename, "profile.toml");
    assert_eq!(add.affected_path, "profiles/code/profile.toml");
    assert_eq!(add.target_kind, "skill");
    assert_eq!(add.target_key, "security");
    assert_eq!(add.operation, "add");
    assert!(add.rule_id.is_none());

    let reloaded = Profile::load_from_dir(fixture.profile_dir()).expect("profile reloads");
    assert_eq!(
        reloaded.config().skills.paths,
        vec!["/root/.codex/skills/security/SKILL.md".to_string()]
    );

    let mut profile = reloaded;
    let edit = profile
        .edit_skill_path("security", "/root/.codex/skills/review/SKILL.md", "ui")
        .expect("skill edit mutation succeeds");
    assert_eq!(edit.target_key, "review");
    assert_eq!(edit.operation, "edit");

    let reloaded = Profile::load_from_dir(fixture.profile_dir()).expect("profile reloads");
    assert_eq!(
        reloaded.config().skills.paths,
        vec!["/root/.codex/skills/review/SKILL.md".to_string()]
    );

    let mut profile = reloaded;
    let delete = profile
        .delete_skill("review", "ui")
        .expect("skill delete mutation succeeds");
    assert_eq!(delete.target_kind, "skill");
    assert_eq!(delete.target_key, "review");
    assert_eq!(delete.operation, "delete");

    let reloaded = Profile::load_from_dir(fixture.profile_dir()).expect("profile reloads");
    assert!(reloaded.config().skills.paths.is_empty());
}

#[test]
fn profile_mcp_tool_permission_override_wins_after_default_mutation() {
    let fixture = ProfileFixture::new();
    let mut profile = Profile::load_from_dir(fixture.profile_dir()).expect("profile loads");
    profile
        .set_mcp_default_permission(SecurityRuleAction::Block, "ui")
        .expect("default MCP mutation succeeds");
    profile
        .set_mcp_tool_permission("capsem", "fetch_http", SecurityRuleAction::Allow, "ui")
        .expect("managed MCP tool override succeeds");

    let reloaded = Profile::load_from_dir(fixture.profile_dir()).expect("profile reloads");
    let permission = reloaded
        .mcp_tool_permission("capsem", "fetch_http")
        .expect("managed MCP permission resolves");
    assert_eq!(permission.action, SecurityRuleAction::Allow);
    assert_eq!(permission.source, "profile_managed");
    assert_eq!(
        permission.rule_id.as_deref(),
        Some("profiles.rules.mcp_capsem_fetch_http_permission")
    );
}

#[test]
fn profile_mcp_tool_permission_mutation_updates_existing_managed_rule() {
    let fixture = ProfileFixture::new();
    let mut profile = Profile::load_from_dir(fixture.profile_dir()).expect("profile loads");
    profile
        .set_mcp_tool_permission("capsem", "fetch_http", SecurityRuleAction::Ask, "ui")
        .expect("first mutation succeeds");
    profile
        .set_mcp_tool_permission("capsem", "fetch_http", SecurityRuleAction::Block, "ui")
        .expect("second mutation updates existing managed rule");

    let rules = profile
        .config()
        .security_rule_profile_from_files(profile.config_root())
        .expect("rules parse");
    let matches = rules
        .profiles
        .rules
        .values()
        .filter(|rule| {
            matches!(
                rule.managed,
                Some(SecurityRuleManagedTarget::McpTool {
                    ref server,
                    ref tool,
                    operation: SecurityRuleManagedOperation::Permission,
                }) if server == "capsem" && tool == "fetch_http"
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].action, SecurityRuleAction::Block);
}

#[test]
fn profile_mcp_tool_permission_requires_pinned_enforcement_file() {
    let fixture = ProfileFixture::new();
    let mut config = Profile::load_from_dir(fixture.profile_dir())
        .unwrap()
        .config()
        .clone();
    config.files.enforcement = None;
    let mut profile = Profile::from_config(
        fixture.config_root(),
        fixture.profile_dir().to_path_buf(),
        config,
    )
    .expect("profile without enforcement pin can still parse before mutation");

    let error = profile
        .set_mcp_tool_permission("capsem", "fetch_http", SecurityRuleAction::Ask, "ui")
        .expect_err("mutation requires enforcement pin");
    assert!(error.contains("profile.files.enforcement"), "{error}");
}

#[test]
fn profile_mcp_tool_permission_rejects_duplicate_managed_targets() {
    let fixture = ProfileFixture::new();
    let managed = r#"
[profiles.rules.first]
name = "first"
action = "ask"
match = 'mcp.server.name == "capsem"'

[profiles.rules.first.managed]
kind = "mcp_tool"
server = "capsem"
tool = "fetch_http"
operation = "permission"

[profiles.rules.second]
name = "second"
action = "block"
match = 'mcp.tool_call.name == "fetch_http"'

[profiles.rules.second.managed]
kind = "mcp_tool"
server = "capsem"
tool = "fetch_http"
operation = "permission"
"#;
    let enforcement = fixture.config_root().join("profiles/code/enforcement.toml");
    std::fs::write(&enforcement, managed).unwrap();
    fixture.repin(
        "enforcement",
        "profiles/code/enforcement.toml",
        &enforcement,
    );

    let mut profile = Profile::load_from_dir(fixture.profile_dir()).expect("profile loads");
    let error = profile
        .set_mcp_tool_permission("capsem", "fetch_http", SecurityRuleAction::Ask, "ui")
        .expect_err("duplicate managed targets are rejected");
    assert!(error.contains("managed security rule target"), "{error}");
}

#[test]
fn checked_in_code_profile_rule_files_compile_into_security_rule_set() {
    let profile = ProfileConfigFile::builtin_primary();
    let config_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../config");
    let rules = profile
        .compile_security_rule_set_from_files(&config_root, SecurityRuleSource::User)
        .expect("profile rule files compile through SecurityRuleSet");
    let rule_ids = rules
        .rules()
        .iter()
        .map(|rule| rule.rule_id.as_str())
        .collect::<Vec<_>>();

    assert!(
        rule_ids.contains(&"profiles.rules.default_http"),
        "default HTTP rule from profile enforcement file must compile"
    );
    assert!(
        rule_ids.contains(&"profiles.rules.skill_loaded"),
        "Sigma detection file must compile into profile security rules"
    );
    assert!(
        rule_ids
            .iter()
            .all(|rule_id| !rule_id.starts_with("policy.")),
        "profile rule files must not mirror into old policy rails"
    );
    assert!(rules
        .rules()
        .iter()
        .all(|rule| !rule.condition.contains("credential.")));
}

#[test]
fn profile_rule_files_reject_old_policy_syntax_and_corp_rules() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("old.toml"),
        r#"
[__OLD_TABLE__]
domains = ["example.com"]
"#
        .replace("__OLD_TABLE__", &("policy".to_string() + ".http")),
    )
    .unwrap();
    let mut profile = ProfileConfigFile::builtin_primary();
    profile.rule_files.enforcement = Some("old.toml".to_string());
    profile.rule_files.sigma = None;
    let error = profile
        .security_rule_profile_from_files(dir.path())
        .expect_err("old policy syntax must not load through profile rule files");
    assert!(error.contains("policy"), "{error}");

    std::fs::write(
        dir.path().join("corp.toml"),
        r#"
[corp.rules.block_example]
name = "block_example"
action = "block"
match = 'http.host == "example.com"'
"#,
    )
    .unwrap();
    profile.rule_files.enforcement = Some("corp.toml".to_string());
    let error = profile
        .security_rule_profile_from_files(dir.path())
        .expect_err("profile rule files cannot smuggle corp ownership");
    assert!(error.contains("must not define corp.rules"), "{error}");
}

#[test]
fn profile_assets_reject_release_manifest_theater_and_build_knobs() {
    let profile = include_str!("../../../../../../config/profiles/code/profile.toml");
    let bad_top_level = profile.replace(
        "refresh_policy = \"on_profile_refresh\"\n",
        "refresh_policy = \"on_profile_refresh\"\nfilesystem = \"erofs\"\n",
    );
    let error = toml::from_str::<ProfileConfigFile>(&bad_top_level)
        .expect_err("profile assets must not expose build filesystem metadata");
    assert!(error.to_string().contains("filesystem"), "{error}");

    let bad_asset = profile.replace(
        "url = \"https://github.com/google/capsem/releases/download/v1.0.1780954707/arm64-vmlinuz\"\n",
        "url = \"https://github.com/google/capsem/releases/download/v1.0.1780954707/arm64-vmlinuz\"\nsignature = \"not-supported\"\n",
    );
    let error = toml::from_str::<ProfileConfigFile>(&bad_asset)
        .expect_err("profile assets must not pretend to carry per-asset signatures");
    assert!(error.to_string().contains("signature"), "{error}");

    let bad_content_type = profile.replace(
        "url = \"https://github.com/google/capsem/releases/download/v1.0.1780954707/arm64-vmlinuz\"\n",
        "url = \"https://github.com/google/capsem/releases/download/v1.0.1780954707/arm64-vmlinuz\"\ncontent_type = \"application/octet-stream\"\n",
    );
    let error = toml::from_str::<ProfileConfigFile>(&bad_content_type)
        .expect_err("profile assets must not expose downloader content types");
    assert!(error.to_string().contains("content_type"), "{error}");
}

#[test]
fn profile_obom_rejects_bad_hash_and_build_knobs() {
    let profile = include_str!("../../../../../../config/profiles/code/profile.toml");
    let with_obom = format!(
        r#"{profile}

[obom]
format = "cyclonedx-obom.v1"

[obom.arch.arm64]
name = "obom.cdx.json"
url = "https://example.invalid/arm64-obom.cdx.json"
hash = "blake3:not-a-real-hash"
size = 10
generator = "cdxgen"
generator_version = "11.0.0"
"#
    );
    let parsed = toml::from_str::<ProfileConfigFile>(&with_obom).expect("obom profile parses");
    let error = parsed.validate().expect_err("bad OBOM hash rejected");
    assert!(error.contains("profile.obom.arch.arm64.hash"), "{error}");

    let bad_format = with_obom.replace("format = \"cyclonedx-obom.v1\"", "format = \"spdx-json\"");
    let parsed = toml::from_str::<ProfileConfigFile>(&bad_format).expect("bad format parses");
    let error = parsed.validate().expect_err("bad OBOM format rejected");
    assert!(error.contains("profile.obom.format"), "{error}");

    let with_build_knob = with_obom.replace(
        "generator_version = \"11.0.0\"\n",
        "generator_version = \"11.0.0\"\ncompression = \"lz4hc\"\n",
    );
    let error = toml::from_str::<ProfileConfigFile>(&with_build_knob)
        .expect_err("OBOM must not expose build knobs");
    assert!(error.to_string().contains("compression"), "{error}");
}

#[test]
fn profile_catalog_loads_directory_profiles_and_rejects_id_mismatch() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("code")).unwrap();
    std::fs::write(
        dir.path().join("code/profile.toml"),
        include_str!("../../../../../../config/profiles/code/profile.toml"),
    )
    .unwrap();

    let catalog = ProfileCatalog::load_from_dir(dir.path()).expect("catalog loads");
    let profile = catalog.get("code").expect("code profile exists");
    assert_eq!(profile.name, "Code");
    assert_eq!(catalog.profiles().count(), 1);

    std::fs::write(
        dir.path().join("legacy-flat.toml"),
        include_str!("../../../../../../config/profiles/code/profile.toml"),
    )
    .unwrap();
    let catalog = ProfileCatalog::load_from_dir(dir.path()).expect("flat files are ignored");
    assert_eq!(catalog.profiles().count(), 1);

    std::fs::create_dir(dir.path().join("wrong")).unwrap();
    std::fs::write(
        dir.path().join("wrong/profile.toml"),
        include_str!("../../../../../../config/profiles/code/profile.toml"),
    )
    .unwrap();
    let error = ProfileCatalog::load_from_dir(dir.path()).unwrap_err();
    assert!(error.contains("id mismatch"), "{error}");
}

#[test]
fn profile_catalog_rejects_flat_only_profile_files() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("code.toml"),
        include_str!("../../../../../../config/profiles/code/profile.toml"),
    )
    .unwrap();

    let error = ProfileCatalog::load_from_dir(dir.path()).unwrap_err();

    assert!(
        error.contains("contains no profile directories with profile.toml"),
        "{error}"
    );
}

struct ProfileFixture {
    dir: tempfile::TempDir,
}

impl ProfileFixture {
    fn new() -> Self {
        let fixture = Self::new_without_downloaded_assets();
        let profile = Profile::load_from_dir(fixture.profile_dir()).expect("profile loads");
        profile
            .download_assets(&fixture.assets_dir(), "arm64")
            .expect("fixture assets download");
        fixture
    }

    fn new_without_downloaded_assets() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let config_root = dir.path().join("config");
        let profile_dir = config_root.join("profiles/code");
        let source_dir = dir.path().join("asset-source/arm64");
        std::fs::create_dir_all(&profile_dir).unwrap();
        std::fs::create_dir_all(&source_dir).unwrap();

        let enforcement = profile_dir.join("enforcement.toml");
        let detection = profile_dir.join("detection.yaml");
        let mcp = profile_dir.join("mcp.json");
        std::fs::write(
            &enforcement,
            r#"
[default.http]
name = "http"
action = "allow"
priority = "default"
reason = "Default allow HTTP."
match = 'has(http.host)'

[default.mcp]
name = "mcp"
action = "allow"
priority = "default"
reason = "Default allow MCP."
match = 'has(mcp.server.name)'
"#,
        )
        .unwrap();
        std::fs::write(
            &detection,
            r#"
title: Skill Loaded
logsource:
  product: capsem
  service: security_event
detection:
  selection:
    file.read.path: /root/.codex/skills/security/SKILL.md
  condition: selection
level: informational
"#,
        )
        .unwrap();
        std::fs::write(
            &mcp,
            r#"{"mcpServers":{"capsem":{"command":"/run/capsem-mcp-server"}}}"#,
        )
        .unwrap();

        let kernel = source_dir.join("vmlinuz");
        let initrd = source_dir.join("initrd.img");
        let rootfs = source_dir.join("rootfs.erofs");
        std::fs::write(&kernel, b"kernel").unwrap();
        std::fs::write(&initrd, b"initrd").unwrap();
        std::fs::write(&rootfs, b"rootfs").unwrap();

        let profile = format!(
            r#"
id = "code"
name = "Code"
description = "Optimized for coding and long-running agents."
revision = "test.1"
refresh_policy = "24h"

[assets]
format = "profile-assets.v1"
refresh_policy = "on_profile_refresh"

[assets.arch.arm64.kernel]
name = "vmlinuz"
url = "file://{}"
hash = "{}"
size = {}

[assets.arch.arm64.initrd]
name = "initrd.img"
url = "file://{}"
hash = "{}"
size = {}

[assets.arch.arm64.rootfs]
name = "rootfs.erofs"
url = "file://{}"
hash = "{}"
size = {}

[rule_files]
enforcement = "profiles/code/enforcement.toml"
sigma = "profiles/code/detection.yaml"

[files.enforcement]
path = "profiles/code/enforcement.toml"
hash = "{}"
size = {}

[files.detection]
path = "profiles/code/detection.yaml"
hash = "{}"
size = {}

[files.mcp]
path = "profiles/code/mcp.json"
hash = "{}"
size = {}

[plugins.credential_broker]
mode = "rewrite"
detectiOn_level = "informational"

[mcp]
health_check_interval_secs = 60

[mcp.server_enabled]
capsem = true
"#,
            kernel.display(),
            descriptor_hash(&kernel),
            file_size(&kernel),
            initrd.display(),
            descriptor_hash(&initrd),
            file_size(&initrd),
            rootfs.display(),
            descriptor_hash(&rootfs),
            file_size(&rootfs),
            descriptor_hash(&enforcement),
            file_size(&enforcement),
            descriptor_hash(&detection),
            file_size(&detection),
            descriptor_hash(&mcp),
            file_size(&mcp),
        )
        .replace("detectiOn_level", "detection_level");
        std::fs::write(profile_dir.join("profile.toml"), profile).unwrap();
        Self { dir }
    }

    fn config_root(&self) -> std::path::PathBuf {
        self.dir.path().join("config")
    }

    fn profile_dir(&self) -> std::path::PathBuf {
        self.config_root().join("profiles/code")
    }

    fn assets_dir(&self) -> std::path::PathBuf {
        self.dir.path().join("assets")
    }

    fn repin(&self, field: &str, relative_path: &str, path: &std::path::Path) {
        let profile_path = self.profile_dir().join("profile.toml");
        let mut profile = std::fs::read_to_string(&profile_path).unwrap();
        let hash_line = format!("hash = \"{}\"", descriptor_hash(path));
        let size_line = format!("size = {}", file_size(path));
        let section = format!("[files.{field}]\npath = \"{relative_path}\"");
        let start = profile.find(&section).expect("section exists");
        let suffix = &profile[start..];
        let hash_pos = start + suffix.find("hash = ").expect("hash exists");
        let hash_end = hash_pos + profile[hash_pos..].find('\n').unwrap();
        profile.replace_range(hash_pos..hash_end, &hash_line);
        let suffix = &profile[start..];
        let size_pos = start + suffix.find("size = ").expect("size exists");
        let size_end = size_pos
            + profile[size_pos..]
                .find('\n')
                .unwrap_or(profile.len() - size_pos);
        profile.replace_range(size_pos..size_end, &size_line);
        std::fs::write(profile_path, profile).unwrap();
    }
}

fn descriptor_hash(path: &std::path::Path) -> String {
    format!(
        "blake3:{}",
        crate::asset_manager::hash_file(path).expect("hash fixture file")
    )
}

fn file_size(path: &std::path::Path) -> u64 {
    std::fs::metadata(path).unwrap().len()
}
