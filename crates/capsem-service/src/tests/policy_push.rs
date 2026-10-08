//! Policy edits reach running VMs before the mutation route returns.
//!
//! Every edit route writes settings.toml, re-materializes each running
//! session's active policy and waits for each VM to acknowledge the exact
//! bytes it was given. An edit that only rewrote the file left every running
//! VM enforcing the old policy until someone reloaded it.
use super::*;

use crate::mcp_routes::{handle_mcp_default_edit, handle_mcp_tool_edit, McpPermissionEditRequest};
use crate::plugin_routes::{handle_plugin_update, PluginUpdate};
use capsem_core::net::policy_config::{ActivePolicyFile, DetectionLevel, SecurityPluginMode, SecurityRuleAction};

const WIKI_SERVER: &str = "[[mcp.servers]]\nname = \"wiki\"\nurl = \"https://wiki.example.invalid/mcp\"\n";

/// A state whose settings.toml declares the `wiki` MCP server, with one
/// running VM per id. Returns the state, each VM's session directory, and
/// the guards that keep the settings redirect alive.
fn state_with_running_vms(dir: &tempfile::TempDir, ids: &[&str]) -> (Arc<ServiceState>, Vec<PathBuf>, impl Drop) {
    let (env_guard, settings_path, _) = install_empty_settings_env(dir);
    std::fs::write(&settings_path, WIKI_SERVER).unwrap();
    let state = make_asset_state(dir.path().join("assets"));
    let sessions = ids
        .iter()
        .map(|id| {
            let session_dir = dir.path().join("sessions").join(id);
            std::fs::create_dir_all(&session_dir).unwrap();
            insert_fake_instance_with_session_dir(&state, id, std::process::id(), session_dir.clone());
            session_dir
        })
        .collect();
    (state, sessions, env_guard)
}

fn uds_path(state: &ServiceState, id: &str) -> PathBuf {
    state.instances.lock().unwrap()[id].uds_path.clone()
}

fn active_policy(session_dir: &StdPath) -> ActivePolicyFile {
    toml::from_str(&std::fs::read_to_string(session_dir.join("vm/active_policy.toml")).unwrap()).unwrap()
}

fn permission(action: SecurityRuleAction) -> Json<McpPermissionEditRequest> {
    Json(McpPermissionEditRequest { action })
}

/// A plugin edit as a client sends it.
fn block_plugin(detection_level: Option<DetectionLevel>) -> Json<PluginUpdate> {
    let mut update = json!({ "mode": SecurityPluginMode::Block });
    if let Some(level) = detection_level {
        update["detection_level"] = json!(level);
    }
    Json(serde_json::from_value(update).unwrap())
}

#[tokio::test]
async fn every_edit_route_materializes_and_reloads_running_vms() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (state, sessions, _env) = state_with_running_vms(&dir, &["push-vm"]);
    let session_dir = &sessions[0];
    // One IPC connection per mutation route below.
    let process = spawn_fake_process_reload_ack(&uds_path(&state, "push-vm"), 3);

    let _ = handle_mcp_default_edit(State(Arc::clone(&state)), permission(SecurityRuleAction::Block))
        .await
        .expect("mcp default edit");
    assert_eq!(
        active_policy(session_dir).user_rules.default["mcp"].action,
        SecurityRuleAction::Block,
        "the tightened MCP default must be in the running session's active policy"
    );

    let _ = handle_mcp_tool_edit(
        State(Arc::clone(&state)),
        Path(("wiki".to_string(), "search".to_string())),
        permission(SecurityRuleAction::Block),
    )
    .await
    .expect("mcp tool edit");
    let rules = active_policy(session_dir).user_rules.profiles.rules;
    assert_eq!(rules["mcp_wiki_search_permission"].action, SecurityRuleAction::Block);

    let Json(plugin) = handle_plugin_update(
        State(Arc::clone(&state)),
        Path("dummy_pre_eicar".to_string()),
        block_plugin(Some(DetectionLevel::Critical)),
    )
    .await
    .expect("plugin edit");
    assert_eq!(serde_json::to_value(&plugin).unwrap()["config"]["mode"], "block");
    let plugins = active_policy(session_dir).plugins;
    assert_eq!(plugins["dummy_pre_eicar"].mode, SecurityPluginMode::Block);
    assert_eq!(plugins["dummy_pre_eicar"].detection_level, DetectionLevel::Critical);

    let received = tokio::time::timeout(std::time::Duration::from_secs(10), process)
        .await
        .expect("every mutation route must contact the running instance")
        .unwrap();
    assert!(only_reloads(&received, 3), "each edit sends exactly one ReloadConfig");
}

/// An edit the corp config owns is refused, and nothing is written or pushed.
#[tokio::test]
async fn an_edit_of_a_corp_owned_setting_is_refused() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (state, _, _env) = state_with_running_vms(&dir, &[]);
    std::fs::write(
        dir.path().join("corp.toml"),
        "[plugins.dummy_pre_eicar]\nmode = \"block\"\ndetection_level = \"high\"\n",
    )
    .unwrap();
    let settings_before = std::fs::read(dir.path().join("settings.toml")).unwrap();

    let error = handle_plugin_update(
        State(Arc::clone(&state)),
        Path("dummy_pre_eicar".to_string()),
        block_plugin(None),
    )
    .await
    .expect_err("corp owns this plugin");

    assert_eq!(error.status, StatusCode::BAD_REQUEST);
    assert!(
        error.body.error.contains("set by the corp config"),
        "{}",
        error.body.error
    );
    assert_eq!(
        std::fs::read(dir.path().join("settings.toml")).unwrap(),
        settings_before
    );
}

/// An acknowledgement only counts when it names the active policy this push
/// wrote. A VM reporting some other policy -- a concurrent edit's, or a stale
/// one -- has not applied this edit, and the route must say so rather than
/// return success on a bare acknowledgement.
#[tokio::test]
async fn a_reload_acknowledging_another_active_policy_fails_the_push() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (state, _, _env) = state_with_running_vms(&dir, &["stale-ack-vm"]);
    let process = spawn_fake_process(&uds_path(&state, "stale-ack-vm"), 1, |message| {
        let reply = match message {
            ServiceToProcess::ReloadConfig { id } => Some(ProcessToService::ConfigReloadResult {
                id: *id,
                active_policy_digest: Some("blake3:some-other-policy".to_string()),
                error: None,
            }),
            _ => None,
        };
        Box::pin(async move { reply })
    });

    let error = handle_mcp_default_edit(State(Arc::clone(&state)), permission(SecurityRuleAction::Block))
        .await
        .expect_err("an acknowledgement of another policy is not this edit applied");
    assert_eq!(error.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        error
            .body
            .error
            .contains("stale-ack-vm: applied active policy blake3:some-other-policy, expected blake3:"),
        "{}",
        error.body.error
    );
    process.await.unwrap();
}

/// Start edit A, hold its VM acknowledgement, then start edit B. Release A
/// once B is either waiting for A or has already rewritten A's session. The
/// fake VM answers each reload with the digest of the active policy it finds
/// when it answers, as capsem-process does. Returns the session's active
/// policy after both edits.
async fn race_edit_b_against_held_edit_a(
    edit_b: impl FnOnce(Arc<ServiceState>) -> tokio::task::JoinHandle<Result<(), AppError>>,
) -> ActivePolicyFile {
    let dir = tempfile::tempdir().unwrap();
    let (state, sessions, _env) = state_with_running_vms(&dir, &["race-vm"]);
    let active_path = sessions[0].join("vm/active_policy.toml");

    let held = Arc::new(tokio::sync::Notify::new());
    let (release, released) = tokio::sync::oneshot::channel::<()>();
    let released = Arc::new(std::sync::Mutex::new(Some(released)));
    let process = spawn_fake_process(&uds_path(&state, "race-vm"), 2, {
        let (held, active_path) = (Arc::clone(&held), active_path.clone());
        move |message| {
            let ServiceToProcess::ReloadConfig { id } = *message else {
                return Box::pin(async { None });
            };
            let (held, released, active_path) =
                (Arc::clone(&held), released.lock().unwrap().take(), active_path.clone());
            Box::pin(async move {
                if let Some(released) = released {
                    held.notify_one();
                    released.await.unwrap();
                }
                Some(ProcessToService::ConfigReloadResult {
                    id,
                    active_policy_digest: Some(capsem_core::net::policy_config::active_policy_digest(
                        &std::fs::read(&active_path).unwrap(),
                    )),
                    error: None,
                })
            })
        }
    });

    let mut edit_a = tokio::spawn(handle_mcp_tool_edit(
        State(Arc::clone(&state)),
        Path(("wiki".to_string(), "search".to_string())),
        permission(SecurityRuleAction::Block),
    ));
    tokio::select! {
        () = held.notified() => {}
        finished = &mut edit_a => panic!(
            "edit A finished before its VM acknowledged it: {:?}",
            finished.unwrap().err().map(|error| error.body.error)
        ),
    }
    let published_by_a = std::fs::read(&active_path).unwrap();
    let edit_b = edit_b(Arc::clone(&state));
    tokio::select! {
        () = state.policy_mutation.contended.notified() => {}
        () = async {
            while std::fs::read(&active_path).unwrap() == published_by_a {
                tokio::task::yield_now().await;
            }
        } => {}
    }
    release.send(()).unwrap();

    let _ = edit_a.await.unwrap().expect("edit A must be acknowledged as edit A");
    edit_b.await.unwrap().expect("edit B");
    assert!(only_reloads(&process.await.unwrap(), 2));
    active_policy(&sessions[0])
}

/// Two MCP edits: neither may be lost, and B's publication may not be
/// acknowledged as A's.
#[tokio::test]
async fn concurrent_edits_keep_both_updates_and_their_own_acknowledgements() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let active = race_edit_b_against_held_edit_a(|state| {
        tokio::spawn(async move {
            handle_mcp_default_edit(State(state), permission(SecurityRuleAction::Block))
                .await
                .map(drop)
        })
    })
    .await;
    assert!(active
        .user_rules
        .profiles
        .rules
        .contains_key("mcp_wiki_search_permission"));
    assert_eq!(active.user_rules.default["mcp"].action, SecurityRuleAction::Block);
}

/// A plugin edit through another route is serialized with an MCP edit.
#[tokio::test]
async fn a_plugin_edit_is_serialized_with_a_concurrent_mcp_edit() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let active = race_edit_b_against_held_edit_a(|state| {
        tokio::spawn(async move {
            handle_plugin_update(State(state), Path("dummy_pre_eicar".to_string()), block_plugin(None))
                .await
                .map(drop)
        })
    })
    .await;
    assert!(active
        .user_rules
        .profiles
        .rules
        .contains_key("mcp_wiki_search_permission"));
    assert_eq!(active.plugins["dummy_pre_eicar"].mode, SecurityPluginMode::Block);
}

/// One running VM whose session cannot take the new policy must not keep the
/// others on the old one. The route applies the edit everywhere it can and
/// says exactly where it could not, without pretending the saved edit was
/// rolled back.
#[tokio::test]
async fn partial_apply_reaches_every_healthy_vm_and_names_the_failed_one() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (state, sessions, _env) = state_with_running_vms(&dir, &["healthy-vm", "broken-vm"]);
    // A session whose `vm/` directory is a file: its active policy cannot be written.
    std::fs::write(sessions[1].join("vm"), b"not a directory").unwrap();
    let healthy = spawn_fake_process_reload_ack(&uds_path(&state, "healthy-vm"), 1);

    let error = handle_mcp_default_edit(State(Arc::clone(&state)), permission(SecurityRuleAction::Block))
        .await
        .expect_err("an edit that did not reach every running VM is not a success");

    assert_eq!(error.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        error
            .body
            .error
            .starts_with("edit saved and recorded; policy applied to 1 of 2 running VMs; not applied: broken-vm: "),
        "{}",
        error.body.error
    );
    assert!(only_reloads(&healthy.await.unwrap(), 1), "the healthy VM is reloaded");
    assert_eq!(
        active_policy(&sessions[0]).user_rules.default["mcp"].action,
        SecurityRuleAction::Block
    );
}

/// An edit is recorded in the host ledger with what changed and where.
#[tokio::test]
async fn an_edit_is_recorded_in_the_host_ledger() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (state, _, _env) = state_with_running_vms(&dir, &[]);

    let Json(response) = handle_mcp_tool_edit(
        State(Arc::clone(&state)),
        Path(("wiki".to_string(), "search".to_string())),
        permission(SecurityRuleAction::Ask),
    )
    .await
    .expect("mcp tool edit");

    let mutation = &response["mutation"];
    assert_eq!(mutation["target_kind"], "mcp_tool");
    assert_eq!(mutation["target_key"], "wiki/search");
    assert_eq!(mutation["operation"], "permission");
    assert_eq!(mutation["affected_path"], "settings.toml");
    assert_eq!(mutation["rule_id"], "profiles.rules.mcp_wiki_search_permission");
    assert_ne!(mutation["old_hash"], mutation["new_hash"]);
    let settings = std::fs::read_to_string(dir.path().join("settings.toml")).unwrap();
    assert!(settings.contains("mcp_wiki_search_permission"), "{settings}");
}

/// A tool of an MCP server nobody declared cannot be given a permission.
#[tokio::test]
async fn a_tool_of_an_undeclared_server_is_refused() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (state, _, _env) = state_with_running_vms(&dir, &[]);

    let error = handle_mcp_tool_edit(
        State(Arc::clone(&state)),
        Path(("nowhere".to_string(), "search".to_string())),
        permission(SecurityRuleAction::Allow),
    )
    .await
    .expect_err("undeclared server");

    assert_eq!(error.status, StatusCode::BAD_REQUEST);
}
