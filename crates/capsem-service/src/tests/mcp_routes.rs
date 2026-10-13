//! The MCP routes answer from settings.toml `[mcp]` with corp laid over it,
//! and write tool permissions back to settings.toml.
use super::*;

/// settings.toml declares the `wiki` server; the local builtin server has
/// one discovered tool. Returns the router and the guards.
fn mcp_app(dir: &tempfile::TempDir, corp: &str) -> (Arc<ServiceState>, Router, impl Drop, EnvVarGuard) {
    let (env_guard, settings_path, corp_path) = install_empty_settings_env(dir);
    std::fs::write(
        &settings_path,
        "[[mcp.servers]]\nname = \"wiki\"\nurl = \"https://wiki.example.invalid/mcp\"\n",
    )
    .unwrap();
    std::fs::write(&corp_path, corp).unwrap();
    let home_guard = EnvVarGuard::set("CAPSEM_HOME", dir.path());
    capsem_core::mcp::save_tool_cache(&[capsem_core::mcp::ToolCacheEntry {
        namespaced_name: "local__fetch_http".to_string(),
        original_name: "fetch_http".to_string(),
        description: Some("Fetch HTTP".to_string()),
        server_name: "local".to_string(),
        annotations: None,
        pin_hash: "tool-pin".to_string(),
        first_seen: "2026-06-10T00:00:00Z".to_string(),
        last_seen: "2026-06-10T00:00:00Z".to_string(),
        approved: true,
    }])
    .expect("write test MCP tool cache");
    let state = make_asset_state(dir.path().join("assets"));
    let app = build_service_router(Arc::clone(&state));
    (state, app, env_guard, home_guard)
}

async fn get(app: &Router, uri: &str) -> serde_json::Value {
    let (status, body) = route_request(app.clone(), axum::http::Method::GET, uri, None).await;
    assert_eq!(status, StatusCode::OK, "{uri}: {body}");
    body
}

#[tokio::test]
async fn mcp_reads_list_the_configured_and_builtin_servers() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    ensure_test_builtin_mcp_binary();
    let dir = tempfile::tempdir().unwrap();
    let (_state, app, _env, _home) = mcp_app(&dir, "");

    let info = get(&app, "/mcp/info").await;
    assert_eq!(info["manual_server_count"], 1);
    assert_eq!(info["builtin_local_enabled"], true);
    assert_eq!(info["server_count"], 2);

    let servers = get(&app, "/mcp/servers/list").await;
    let source_of = |name: &str| {
        servers
            .as_array()
            .unwrap()
            .iter()
            .find(|server| server["name"] == name)
            .map(|server| server["source"].clone())
    };
    assert_eq!(source_of("wiki"), Some(json!("config")));
    assert_eq!(source_of("local"), Some(json!("builtin")));

    let default = get(&app, "/mcp/default/info").await;
    assert_eq!(default["source"], "default");
    assert_eq!(default["rule_id"], "default.mcp");

    let (status, refresh) =
        route_request(app.clone(), axum::http::Method::POST, "/mcp/servers/wiki/refresh", None).await;
    assert_eq!(status, StatusCode::OK, "{refresh}");
    assert_eq!(refresh["success"], true);
    assert_eq!(refresh["instances"], 0, "no running VM, nothing contacted");
}

#[tokio::test]
async fn a_tool_permission_edit_is_written_to_settings_and_recorded() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (state, app, _env, _home) = mcp_app(&dir, "");

    let (status, edited) = route_request(
        app.clone(),
        axum::http::Method::PATCH,
        "/mcp/servers/local/tools/fetch_http/edit",
        Some(json!({ "action": "ask" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{edited}");
    assert_eq!(edited["server_id"], "local");
    assert_eq!(edited["tool_id"], "fetch_http");
    assert_eq!(edited["action"], "ask");
    assert_eq!(edited["mutation"]["category"], "mcp");
    assert_eq!(edited["mutation"]["status"], "applied");

    let settings = capsem_core::net::policy_config::load_settings_file(&dir.path().join("settings.toml")).unwrap();
    let rule = &settings.profiles.rules["mcp_local_fetch_http_permission"];
    assert_eq!(rule.action, SecurityRuleAction::Ask);
    assert_eq!(
        rule.condition,
        r#"mcp.server.name == "local" && mcp.tool_call.name == "fetch_http""#
    );

    let tools = get(&app, "/mcp/servers/local/tools/list").await;
    assert_eq!(tools[0]["namespaced_name"], "local__fetch_http");
    assert_eq!(tools[0]["permission_action"], "ask");
    assert_eq!(tools[0]["permission_source"], "settings");

    state.host_ledger.flush().await.expect("flush host ledger");
    let reader = capsem_logger::DbReader::open(state.host_ledger.path()).expect("host ledger");
    let rows: serde_json::Value = serde_json::from_str(
        &reader
            .query_raw(
                "SELECT category, target_kind, target_key, operation, affected_path, status \
                 FROM policy_mutation_events",
            )
            .expect("query policy mutation events"),
    )
    .unwrap();
    assert_eq!(
        rows["rows"][0],
        json!([
            "mcp",
            "mcp_tool",
            "local/fetch_http",
            "permission",
            "settings.toml",
            "applied"
        ])
    );
}

#[tokio::test]
async fn the_default_permission_edit_takes_effect_unless_corp_sets_it() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (_state, app, _env, _home) = mcp_app(&dir, "");

    let (status, body) = route_request(
        app.clone(),
        axum::http::Method::PATCH,
        "/mcp/default/edit",
        Some(json!({ "action": "block" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let default = get(&app, "/mcp/default/info").await;
    assert_eq!(default["action"], "block");
    assert_eq!(default["source"], "settings");

    let corp_dir = tempfile::tempdir().unwrap();
    drop((_state, app, _env, _home));
    let corp_default =
        "[default.mcp]\nname = \"mcp\"\naction = \"allow\"\npriority = \"default\"\nmatch = 'has(mcp.method)'\n";
    let (_state, app, _env, _home) = mcp_app(&corp_dir, corp_default);
    let (status, refused) = route_request(
        app.clone(),
        axum::http::Method::PATCH,
        "/mcp/default/edit",
        Some(json!({ "action": "block" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    assert!(refused["error"].as_str().unwrap().contains("corp"), "{refused}");
    assert_eq!(get(&app, "/mcp/default/info").await["source"], "corp");
}

#[tokio::test]
async fn an_unconfigured_server_is_not_found() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (_state, app, _env, _home) = mcp_app(&dir, "");

    for (method, uri, body) in [
        (axum::http::Method::GET, "/mcp/servers/nowhere/tools/list", None),
        (axum::http::Method::POST, "/mcp/servers/nowhere/refresh", None),
        (
            axum::http::Method::POST,
            "/mcp/servers/nowhere/tools/search/call",
            Some(json!({})),
        ),
    ] {
        let (status, error) = route_request(app.clone(), method, uri, body).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}: {error}");
        assert!(
            error["error"]
                .as_str()
                .unwrap()
                .contains("MCP server not configured: nowhere"),
            "{error}"
        );
    }

    insert_fake_instance_with_session_dir(&_state, "box", 1, dir.path().join("session"));
    let (status, _error) = route_request(
        app.clone(),
        axum::http::Method::POST,
        "/mcp/servers/local/tools/fetch_http/call",
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
}

#[test]
fn a_tool_id_must_belong_to_its_server() {
    assert_eq!(
        crate::mcp_routes::resolve_mcp_tool_id("wiki", "search").unwrap(),
        "wiki__search"
    );
    assert_eq!(
        crate::mcp_routes::resolve_mcp_tool_id("wiki", "wiki__search").unwrap(),
        "wiki__search"
    );
    let error = crate::mcp_routes::resolve_mcp_tool_id("wiki", "notes__search").unwrap_err();
    assert_eq!(error.status, StatusCode::BAD_REQUEST);
}
