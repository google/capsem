use super::*;

// -----------------------------------------------------------------------
// Settings tree tests
// -----------------------------------------------------------------------

#[test]
fn settings_tree_has_top_level_groups() {
    let resolved = resolve_settings(&empty_file(), &empty_file());
    let tree = build_settings_tree(&resolved);
    assert!(!tree.is_empty(), "tree should have top-level nodes");
    // All top-level nodes should be groups
    for node in &tree {
        match node {
            SettingsNode::Group { name, .. } => {
                assert!(!name.is_empty());
            }
            SettingsNode::Leaf(_) => {
                panic!("top-level nodes should be groups, not leaves");
            }
            SettingsNode::Action { .. } => {
                // Action nodes can appear at top level
            }
        }
    }
}

#[test]
fn settings_tree_contains_all_definitions() {
    let resolved = resolve_settings(&empty_file(), &empty_file());
    let tree = build_settings_tree(&resolved);
    let defs = setting_definitions();

    fn collect_leaf_ids(nodes: &[SettingsNode]) -> Vec<String> {
        let mut ids = Vec::new();
        for node in nodes {
            match node {
                SettingsNode::Leaf(s) => ids.push(s.id.clone()),
                SettingsNode::Group { children, .. } => {
                    ids.extend(collect_leaf_ids(children));
                }
                SettingsNode::Action { .. } => {}
            }
        }
        ids
    }

    let leaf_ids = collect_leaf_ids(&tree);
    for def in &defs {
        assert!(leaf_ids.contains(&def.id), "tree missing definition: {}", def.id,);
    }
}

#[test]
fn settings_tree_groups_have_expected_names() {
    let resolved = resolve_settings(&empty_file(), &empty_file());
    let tree = build_settings_tree(&resolved);

    fn collect_group_names(nodes: &[SettingsNode]) -> Vec<String> {
        let mut names = Vec::new();
        for node in nodes {
            if let SettingsNode::Group { name, children, .. } = node {
                names.push(name.clone());
                names.extend(collect_group_names(children));
            }
        }
        names
    }

    let names = collect_group_names(&tree);
    for expected in &[
        "Security",
        "Network Mechanics",
        "Services",
        "Search Engines",
        "Package Registries",
        "Appearance",
        "VM",
        "Environment",
        "Resources",
    ] {
        assert!(names.contains(&expected.to_string()), "tree missing group: {expected}",);
    }
}

#[test]
fn settings_tree_serializes_to_json() {
    let resolved = resolve_settings(&empty_file(), &empty_file());
    let tree = build_settings_tree(&resolved);
    let json = serde_json::to_string(&tree).unwrap();
    // Verify it round-trips
    let _: Vec<SettingsNode> = serde_json::from_str(&json).unwrap();
    assert!(json.contains("\"kind\":\"group\""));
    assert!(json.contains("\"kind\":\"leaf\""));
}

#[test]
fn settings_tree_dynamic_env_appended_to_guest() {
    let user = file_with(vec![("guest.env.EDITOR", SettingValue::Text("vim".into()))]);
    let resolved = resolve_settings(&user, &empty_file());
    let tree = build_settings_tree(&resolved);

    fn find_leaf_in_group(nodes: &[SettingsNode], group_name: &str, leaf_id: &str) -> bool {
        for node in nodes {
            if let SettingsNode::Group { name, children, .. } = node {
                if name == group_name {
                    return children.iter().any(|c| match c {
                        SettingsNode::Leaf(s) => s.id == leaf_id,
                        SettingsNode::Group { children, .. } => children.iter().any(|cc| match cc {
                            SettingsNode::Leaf(s) => s.id == leaf_id,
                            _ => false,
                        }),
                        _ => false,
                    });
                }
                if find_leaf_in_group(children, group_name, leaf_id) {
                    return true;
                }
            }
        }
        false
    }

    assert!(
        find_leaf_in_group(&tree, "Environment", "guest.env.EDITOR"),
        "dynamic guest.env.EDITOR should appear in Environment group (under VM)",
    );
}

#[test]
fn settings_tree_enabled_by_on_groups() {
    let resolved = resolve_settings(&empty_file(), &empty_file());
    let tree = build_settings_tree(&resolved);

    fn find_group(nodes: &[SettingsNode], key: &str) -> Option<SettingsNode> {
        for node in nodes {
            if let SettingsNode::Group { key: k, children, .. } = node {
                if k == key {
                    return Some(node.clone());
                }
                if let Some(found) = find_group(children, key) {
                    return Some(found);
                }
            }
        }
        None
    }

    let github = find_group(&tree, "repository.providers.github");
    assert!(github.is_some(), "should find repository.providers.github group");
    if let Some(SettingsNode::Group { enabled_by, .. }) = github {
        assert_eq!(enabled_by, Some(SETTING_GITHUB_ALLOW.to_string()));
    }
}

// -----------------------------------------------------------------------
// Grammar: action nodes in tree
// -----------------------------------------------------------------------

#[test]
fn settings_tree_contains_action_nodes() {
    let resolved = resolve_settings(&empty_file(), &empty_file());
    let tree = build_settings_tree(&resolved);

    fn find_action(nodes: &[SettingsNode], action: ActionKind) -> bool {
        for node in nodes {
            match node {
                SettingsNode::Action { action: a, .. } if *a == action => return true,
                SettingsNode::Group { children, .. } => {
                    if find_action(children, action) {
                        return true;
                    }
                }
                _ => {}
            }
        }
        false
    }

    assert!(
        find_action(&tree, ActionKind::CheckUpdate),
        "tree should contain check_update action"
    );
}

#[test]
fn action_nodes_not_in_setting_definitions() {
    let defs = setting_definitions();
    // Action node keys should NOT appear as setting definitions
    assert!(
        defs.iter().all(|d| d.id != "app.check_update"),
        "action nodes should not be in setting_definitions"
    );
}

// -----------------------------------------------------------------------
// Grammar: side_effect metadata
// -----------------------------------------------------------------------

#[test]
fn dark_mode_has_side_effect() {
    let defs = setting_definitions();
    let dark_mode = defs.iter().find(|d| d.id == "appearance.dark_mode").unwrap();
    assert_eq!(dark_mode.metadata.side_effect, Some(SideEffect::ToggleTheme));
}

// -----------------------------------------------------------------------
// Grammar: list value types
// -----------------------------------------------------------------------

#[test]
fn setting_value_string_list_roundtrip() {
    let val = SettingValue::StringList(vec!["a.com".into(), "b.com".into()]);
    let json = serde_json::to_string(&val).unwrap();
    let back: SettingValue = serde_json::from_str(&json).unwrap();
    assert_eq!(val, back);
}

#[test]
fn setting_value_int_list_roundtrip() {
    let val = SettingValue::IntList(vec![1, 2, 3]);
    let json = serde_json::to_string(&val).unwrap();
    let back: SettingValue = serde_json::from_str(&json).unwrap();
    assert_eq!(val, back);
}

#[test]
fn setting_value_float_list_roundtrip() {
    let val = SettingValue::FloatList(vec![1.5, 2.5]);
    let json = serde_json::to_string(&val).unwrap();
    let back: SettingValue = serde_json::from_str(&json).unwrap();
    assert_eq!(val, back);
}

// -----------------------------------------------------------------------
// Batch update + corp enforcement
// -----------------------------------------------------------------------

pub(super) fn with_temp_configs<F: FnOnce(&std::path::Path, &std::path::Path)>(
    user_entries: Vec<(&str, SettingValue)>,
    corp_entries: Vec<(&str, SettingValue)>,
    f: F,
) {
    // This helper mutates process-wide env vars that the loader reads.
    // Serialize across the whole test binary so parallel tests don't
    // stomp each other's CAPSEM_*_CONFIG (caused flaky batch_update_*
    // failures before this lock).
    let _guard = crate::credential_broker::TEST_ENV_LOCK.blocking_lock();

    let dir = tempfile::tempdir().unwrap();
    let user_path = dir.path().join("settings.toml");
    let corp_path = dir.path().join("corp.toml");
    let user_file = file_with(user_entries);
    let corp_file = file_with(corp_entries);
    loader::write_settings_file(&user_path, &user_file).unwrap();
    loader::write_settings_file(&corp_path, &corp_file).unwrap();
    // Point env vars to temp files
    let _capsem_paths = capsem_foundation::paths::CapsemPathsGuard::redirect(dir.path());
    std::env::set_var("CAPSEM_CORP_CONFIG", &corp_path);
    f(&user_path, &corp_path);
    std::env::remove_var("CAPSEM_CORP_CONFIG");
}

#[test]
fn batch_update_accepts_valid_changes() {
    with_temp_configs(vec![], vec![], |_, _| {
        let mut changes = HashMap::new();
        changes.insert("appearance.dark_mode".to_string(), SettingValue::Bool(true));
        let result = loader::batch_update_settings(&changes);
        assert!(result.is_ok(), "valid changes should succeed: {:?}", result);
        let applied = result.unwrap();
        assert_eq!(applied, vec!["appearance.dark_mode"]);
    });
}

#[test]
fn batch_update_rejects_profile_behavior_settings() {
    with_temp_configs(vec![], vec![], |_, _| {
        let mut changes = HashMap::new();
        changes.insert(SETTING_GITHUB_ALLOW.to_string(), SettingValue::Bool(true));
        let result = loader::batch_update_settings(&changes);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("profile-owned setting"));
    });
}

#[test]
fn batch_update_rejects_mixed_batch_atomically() {
    with_temp_configs(vec![], vec![], |user_path, _| {
        let mut changes = HashMap::new();
        changes.insert("appearance.dark_mode".to_string(), SettingValue::Bool(true));
        changes.insert(SETTING_GITHUB_ALLOW.to_string(), SettingValue::Bool(true));
        let result = loader::batch_update_settings(&changes);
        assert!(result.is_err(), "mixed batch should be rejected");

        // Verify nothing was written (atomic rejection)
        let file = loader::load_settings_file(user_path).unwrap();
        assert!(
            file.settings.is_empty(),
            "valid UI setting should NOT be written when batch is rejected"
        );
    });
}

#[test]
fn batch_update_rejects_unknown_setting_id() {
    with_temp_configs(vec![], vec![], |_, _| {
        let mut changes = HashMap::new();
        changes.insert("nonexistent.setting".to_string(), SettingValue::Bool(true));
        let result = loader::batch_update_settings(&changes);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unknown setting"));
    });
}

#[test]
fn batch_update_settings_rejects_profile_owned_setting_ids() {
    with_temp_configs(vec![], vec![], |_, _| {
        let mut changes = HashMap::new();
        changes.insert("vm.resources.cpu_count".to_string(), SettingValue::Number(8));
        let result = loader::batch_update_settings(&changes);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("profile-owned setting"));
    });
}

#[test]
fn batch_update_rejects_retired_web_decision_setting_ids() {
    with_temp_configs(vec![], vec![], |_, _| {
        let mut changes = HashMap::new();
        for retired_id in [
            "security.web.allow_read",
            "security.web.allow_write",
            "security.web.custom_allow",
            "security.web.custom_block",
        ] {
            changes.insert(retired_id.to_string(), SettingValue::Bool(true));
            let result = loader::batch_update_settings(&changes);
            assert!(result.is_err(), "{retired_id} should be rejected");
            assert!(result.unwrap_err().contains("unknown setting"));
            changes.clear();
        }
    });
}

#[test]
fn batch_update_rejects_dynamic_guest_env() {
    with_temp_configs(vec![], vec![], |_, _| {
        let mut changes = HashMap::new();
        changes.insert("guest.env.MY_VAR".to_string(), SettingValue::Text("hello".into()));
        let result = loader::batch_update_settings(&changes);
        assert!(
            result.is_err(),
            "dynamic guest.env.* belongs to profile/bootstrap, not settings"
        );
        assert!(result.unwrap_err().contains("profile-owned setting"));
    });
}

#[test]
fn batch_update_empty_is_noop() {
    with_temp_configs(vec![], vec![], |_, _| {
        let changes = HashMap::new();
        let result = loader::batch_update_settings(&changes);
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    });
}

#[test]
fn load_settings_response_returns_all_fields() {
    with_temp_configs(vec![], vec![], |_, _| {
        let response = loader::load_settings_response();
        assert!(!response.tree.is_empty(), "tree should not be empty");
        assert!(response
            .issues
            .iter()
            .all(|issue| !issue.id.is_empty() && !issue.message.is_empty()));
    });
}

// -----------------------------------------------------------------------
// .git-credentials generation tests
// -----------------------------------------------------------------------

#[test]
fn git_credentials_not_generated_from_github_token_settings() {
    let user = file_with(vec![
        (SETTING_GITHUB_ALLOW, SettingValue::Bool(true)),
        (
            SETTING_GITHUB_TOKEN,
            SettingValue::Text(
                "credential:blake3:1111111111111111111111111111111111111111111111111111111111111111".into(),
            ),
        ),
    ]);
    let resolved = resolve_settings(&user, &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let files = gc.files.unwrap_or_default();
    assert!(!files.iter().any(|f| f.path == "/root/.git-credentials"));
    assert!(!files.iter().any(|f| f.path == "/root/.gitconfig"));
}

#[test]
fn git_credentials_not_generated_from_multiple_provider_settings() {
    let user = file_with(vec![
        (SETTING_GITHUB_ALLOW, SettingValue::Bool(true)),
        (
            SETTING_GITHUB_TOKEN,
            SettingValue::Text(
                "credential:blake3:1111111111111111111111111111111111111111111111111111111111111111".into(),
            ),
        ),
        (SETTING_GITLAB_ALLOW, SettingValue::Bool(true)),
        (SETTING_GITLAB_TOKEN, SettingValue::Text("glpat-test456".into())),
    ]);
    let resolved = resolve_settings(&user, &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let files = gc.files.unwrap_or_default();
    assert!(!files.iter().any(|f| f.path == "/root/.git-credentials"));
    assert!(!files.iter().any(|f| f.path == "/root/.gitconfig"));
}

#[test]
fn git_credentials_not_generated_when_allow_false() {
    let user = file_with(vec![
        (SETTING_GITHUB_ALLOW, SettingValue::Bool(false)),
        (SETTING_GITHUB_TOKEN, SettingValue::Text("ghp_test123".into())),
    ]);
    let resolved = resolve_settings(&user, &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let has_creds = gc
        .files
        .as_ref()
        .is_some_and(|f| f.iter().any(|f| f.path == "/root/.git-credentials"));
    assert!(!has_creds, ".git-credentials should not be generated when allow=false");
}

#[test]
fn git_credentials_not_generated_when_token_empty() {
    let user = file_with(vec![(SETTING_GITHUB_ALLOW, SettingValue::Bool(true))]);
    let resolved = resolve_settings(&user, &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let has_creds = gc
        .files
        .as_ref()
        .is_some_and(|f| f.iter().any(|f| f.path == "/root/.git-credentials"));
    assert!(
        !has_creds,
        ".git-credentials should not be generated when token is empty"
    );
}

#[test]
fn git_credentials_not_generated_when_corp_blocks() {
    let user = file_with(vec![(SETTING_GITHUB_TOKEN, SettingValue::Text("ghp_test123".into()))]);
    let corp = file_with(vec![(SETTING_GITHUB_ALLOW, SettingValue::Bool(false))]);
    let resolved = resolve_settings(&user, &corp);
    let gc = settings_to_guest_config(&resolved);
    let has_creds = gc
        .files
        .as_ref()
        .is_some_and(|f| f.iter().any(|f| f.path == "/root/.git-credentials"));
    assert!(
        !has_creds,
        ".git-credentials should not be generated when corp blocks provider"
    );
}

#[test]
fn git_credentials_rejects_token_with_special_chars() {
    // Newlines
    let user = file_with(vec![
        (SETTING_GITHUB_ALLOW, SettingValue::Bool(true)),
        (SETTING_GITHUB_TOKEN, SettingValue::Text("ghp_test\ninjected".into())),
    ]);
    let resolved = resolve_settings(&user, &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let has_creds = gc
        .files
        .as_ref()
        .is_some_and(|f| f.iter().any(|f| f.path == "/root/.git-credentials"));
    assert!(
        !has_creds,
        ".git-credentials should not be generated when token contains newlines"
    );

    // @ sign (could inject a different host)
    let user = file_with(vec![
        (SETTING_GITHUB_ALLOW, SettingValue::Bool(true)),
        (SETTING_GITHUB_TOKEN, SettingValue::Text("ghp_test@evil.com".into())),
    ]);
    let resolved = resolve_settings(&user, &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let has_creds = gc
        .files
        .as_ref()
        .is_some_and(|f| f.iter().any(|f| f.path == "/root/.git-credentials"));
    assert!(
        !has_creds,
        ".git-credentials should not be generated when token contains @"
    );

    // : colon (could break URL structure)
    let user = file_with(vec![
        (SETTING_GITHUB_ALLOW, SettingValue::Bool(true)),
        (SETTING_GITHUB_TOKEN, SettingValue::Text("ghp_test:injected".into())),
    ]);
    let resolved = resolve_settings(&user, &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let has_creds = gc
        .files
        .as_ref()
        .is_some_and(|f| f.iter().any(|f| f.path == "/root/.git-credentials"));
    assert!(
        !has_creds,
        ".git-credentials should not be generated when token contains :"
    );
}

#[test]
fn git_credentials_gitconfig_not_generated_without_tokens() {
    // No tokens at all -- neither .git-credentials nor .gitconfig should exist
    let user = file_with(vec![(SETTING_GITHUB_ALLOW, SettingValue::Bool(true))]);
    let resolved = resolve_settings(&user, &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let has_creds = gc
        .files
        .as_ref()
        .is_some_and(|f| f.iter().any(|f| f.path == "/root/.git-credentials"));
    let has_gitconfig = gc
        .files
        .as_ref()
        .is_some_and(|f| f.iter().any(|f| f.path == "/root/.gitconfig"));
    assert!(!has_creds, ".git-credentials should not exist without tokens");
    assert!(!has_gitconfig, ".gitconfig should not exist without tokens");
}

// -----------------------------------------------------------------------
// Git identity env var tests
// -----------------------------------------------------------------------

#[test]
fn git_identity_env_vars_injected() {
    let user = file_with(vec![
        (
            "repository.git.identity.author_name",
            SettingValue::Text("Test User".into()),
        ),
        (
            "repository.git.identity.author_email",
            SettingValue::Text("test@example.com".into()),
        ),
    ]);
    let resolved = resolve_settings(&user, &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let env = gc.env.unwrap();
    assert_eq!(env.get("GIT_AUTHOR_NAME").unwrap(), "Test User");
    assert_eq!(env.get("GIT_COMMITTER_NAME").unwrap(), "Test User");
    assert_eq!(env.get("GIT_AUTHOR_EMAIL").unwrap(), "test@example.com");
    assert_eq!(env.get("GIT_COMMITTER_EMAIL").unwrap(), "test@example.com");
}

#[test]
fn git_identity_env_vars_absent_when_empty() {
    let resolved = resolve_settings(&empty_file(), &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let env = gc.env.unwrap_or_default();
    assert!(
        !env.contains_key("GIT_AUTHOR_NAME"),
        "GIT_AUTHOR_NAME should not be set when empty"
    );
    assert!(
        !env.contains_key("GIT_COMMITTER_NAME"),
        "GIT_COMMITTER_NAME should not be set when empty"
    );
    assert!(
        !env.contains_key("GIT_AUTHOR_EMAIL"),
        "GIT_AUTHOR_EMAIL should not be set when empty"
    );
    assert!(
        !env.contains_key("GIT_COMMITTER_EMAIL"),
        "GIT_COMMITTER_EMAIL should not be set when empty"
    );
}

// -----------------------------------------------------------------------
// Repository section definitions tests
// -----------------------------------------------------------------------

#[test]
fn repository_settings_exist_in_definitions() {
    let defs = setting_definitions();
    let ids = [
        "repository.git.identity.author_name",
        "repository.git.identity.author_email",
        SETTING_GITHUB_ALLOW,
        "repository.providers.github.domains",
        SETTING_GITHUB_TOKEN,
        SETTING_GITLAB_ALLOW,
        "repository.providers.gitlab.domains",
        SETTING_GITLAB_TOKEN,
    ];
    for id in &ids {
        assert!(defs.iter().any(|d| d.id == *id), "missing setting definition: {id}");
    }
}

#[test]
fn default_github_allowed_gitlab_not() {
    let resolved = resolve_settings(&empty_file(), &empty_file());
    let gh = resolved.iter().find(|s| s.id == SETTING_GITHUB_ALLOW).unwrap();
    assert_eq!(gh.effective_value, SettingValue::Bool(true));
    let gl = resolved.iter().find(|s| s.id == SETTING_GITLAB_ALLOW).unwrap();
    assert_eq!(gl.effective_value, SettingValue::Bool(false));
}

#[test]
fn setting_id_constants_exist_in_registry() {
    let defs = setting_definitions();
    let ids: Vec<&str> = defs.iter().map(|d| d.id.as_str()).collect();
    for constant in [
        SETTING_GITHUB_ALLOW,
        SETTING_GITHUB_TOKEN,
        SETTING_GITLAB_ALLOW,
        SETTING_GITLAB_TOKEN,
    ] {
        assert!(
            ids.contains(&constant),
            "constant '{constant}' not found in setting_definitions()"
        );
    }
}

// -----------------------------------------------------------------------
// GH_TOKEN / GITLAB_TOKEN materialization guards
// -----------------------------------------------------------------------

#[test]
fn gh_token_not_materialized_when_github_enabled() {
    let user = file_with(vec![
        (SETTING_GITHUB_ALLOW, SettingValue::Bool(true)),
        (
            SETTING_GITHUB_TOKEN,
            SettingValue::Text(
                "credential:blake3:1111111111111111111111111111111111111111111111111111111111111111".into(),
            ),
        ),
    ]);
    let resolved = resolve_settings(&user, &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let env = gc.env.unwrap_or_default();
    assert!(!env.contains_key("GH_TOKEN"));
    assert!(!env.contains_key("GITHUB_TOKEN"));
}

#[test]
fn gitlab_token_not_materialized_when_gitlab_enabled() {
    let user = file_with(vec![
        (SETTING_GITLAB_ALLOW, SettingValue::Bool(true)),
        (
            SETTING_GITLAB_TOKEN,
            SettingValue::Text(
                "credential:blake3:2222222222222222222222222222222222222222222222222222222222222222".into(),
            ),
        ),
    ]);
    let resolved = resolve_settings(&user, &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let env = gc.env.unwrap_or_default();
    assert!(!env.contains_key("GITLAB_TOKEN"));
}

#[test]
fn gh_token_not_injected_when_token_empty() {
    let user = file_with(vec![(SETTING_GITHUB_ALLOW, SettingValue::Bool(true))]);
    let resolved = resolve_settings(&user, &empty_file());
    let gc = settings_to_guest_config(&resolved);
    let env = gc.env.unwrap_or_default();
    assert!(
        !env.contains_key("GH_TOKEN"),
        "GH_TOKEN should not be set when token is empty"
    );
    assert!(
        !env.contains_key("GITHUB_TOKEN"),
        "GITHUB_TOKEN should not be set when token is empty"
    );
}

// -----------------------------------------------------------------------
// Prefix metadata tests
// -----------------------------------------------------------------------

#[test]
fn token_settings_have_prefix_metadata() {
    let defs = setting_definitions();
    let gh = defs.iter().find(|d| d.id == SETTING_GITHUB_TOKEN).unwrap();
    assert_eq!(gh.metadata.prefix.as_deref(), Some("ghp_"));
    let gl = defs.iter().find(|d| d.id == SETTING_GITLAB_TOKEN).unwrap();
    assert_eq!(gl.metadata.prefix.as_deref(), Some("glpat-"));
}

// -----------------------------------------------------------------------
// Setting ID migration
// -----------------------------------------------------------------------

#[test]
fn migrate_old_setting_ids() {
    let mut file = file_with(vec![
        ("web.defaults.allow_read", SettingValue::Bool(true)),
        ("web.custom_allow", SettingValue::Text("example.com".into())),
        ("registry.npm.allow", SettingValue::Bool(false)),
        ("web.search.google.allow", SettingValue::Bool(true)),
    ]);
    migrate_setting_ids(&mut file);

    // Old keys removed
    assert!(file.settings.contains_key("web.defaults.allow_read"));
    assert!(file.settings.contains_key("web.custom_allow"));
    assert!(!file.settings.contains_key("registry.npm.allow"));
    assert!(!file.settings.contains_key("web.search.google.allow"));

    // Live service keys still migrate; retired web decision keys do not.
    assert!(!file.settings.contains_key("security.web.allow_read"));
    assert!(!file.settings.contains_key("security.web.custom_allow"));
    assert_eq!(
        file.settings["security.services.registry.npm.allow"].value,
        SettingValue::Bool(false)
    );
    assert_eq!(
        file.settings["security.services.search.google.allow"].value,
        SettingValue::Bool(true)
    );
}

#[test]
fn migrate_does_not_clobber_existing_new_keys() {
    let mut file = SettingsFile::default();
    file.settings.insert(
        "web.search.google.allow".to_string(),
        SettingEntry {
            value: SettingValue::Bool(true),
            modified: now_str(),
        },
    );
    file.settings.insert(
        "security.services.search.google.allow".to_string(),
        SettingEntry {
            value: SettingValue::Bool(false),
            modified: now_str(),
        },
    );
    migrate_setting_ids(&mut file);

    // New key keeps its value, old key is dropped
    assert_eq!(
        file.settings["security.services.search.google.allow"].value,
        SettingValue::Bool(false)
    );
    assert!(!file.settings.contains_key("web.search.google.allow"));
}
