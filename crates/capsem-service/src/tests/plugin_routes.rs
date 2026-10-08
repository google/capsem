//! The plugin routes: the catalog, edits, and plugin runtime hydrated from
//! the running sessions' security ledgers.
//!
//! These are the routes that do not just list rule matches but parse their
//! forensic payloads -- to count plugin executions and brokered credentials --
//! so they are the ones that read the archive as well as the row.

use super::*;

fn archived_forensic_json(body: capsem_logger::StoredBody) -> String {
    assert_eq!(
        body.content_type.as_deref(),
        Some("application/vnd.capsem.security+msgpack")
    );
    capsem_proto::forensic::SecurityForensicEvent::decode(&body.bytes)
        .unwrap()
        .to_json()
        .unwrap()
}

#[tokio::test]
async fn credential_broker_reload_route_rehydrates_store_and_returns_same_contract() {
    let _lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let test_store = dir.path().join("credential-store.json");
    let _store_guard = EnvVarGuard::set("CAPSEM_CREDENTIAL_STORE_PATH", test_store.clone());
    let state = make_test_state();
    let app = build_service_router(Arc::clone(&state));
    let session_dir = dir.path().join("sessions").join("broker-reload-vm");
    std::fs::create_dir_all(&session_dir).unwrap();
    insert_fake_instance_with_session_dir(&state, "broker-reload-vm", std::process::id(), session_dir.clone());

    let credential_ref = capsem_logger::credential_reference("google", "ya29.reload-route");
    let store_json = serde_json::json!({
        capsem_core::credential_broker::credential_store_account(
            capsem_core::credential_broker::CredentialProvider::Google,
            &credential_ref,
        ): "ya29.reload-route"
    });
    std::fs::write(&test_store, serde_json::to_string_pretty(&store_json).unwrap()).unwrap();

    let event_json = format!(
        r#"{{
            "event_type": "http.request",
            "credential_observations": [
                {{
                    "provider": "google",
                    "source": "http.body.response.$.access_token",
                    "event_type": "http.request",
                    "trace_id": null,
                    "context_json": {{"domain":"oauth2.googleapis.com"}},
                    "credential_ref": "{credential_ref}"
                }}
            ],
            "credential_injections": []
        }}"#
    );
    let session_db = session_dir.join("session.db");
    let writer = capsem_logger::DbWriter::open(&session_db, 16).unwrap();
    let stale_reader = state
        .register_session_db_handle("broker-reload-vm", &session_dir)
        .expect("pre-register session DB reader");
    writer
        .write(capsem_logger::WriteOp::SecurityRuleEvent(
            capsem_logger::SecurityRuleEvent::new(
                1_789_000_123_456,
                "abcd1234ef56",
                "http.request",
                "profiles.rules.default_http",
                r#"{"name":"default_http"}"#,
                event_json,
            ),
        ))
        .await;
    writer.shutdown_blocking();
    let direct_rows = capsem_logger::DbReader::open(&session_db)
        .unwrap()
        .recent_security_rule_events(10)
        .unwrap();
    assert_eq!(direct_rows.len(), 1);
    // The forensic payload is archive-backed now, so the ledger holds it only
    // if the archive gives it back.
    let payload = capsem_logger::DbHandle::open_external_reader(&session_db)
        .unwrap()
        .read_body(
            "abcd1234ef56",
            "security_rule_events",
            capsem_logger::BodyDirection::Payload,
        )
        .await
        .unwrap()
        .expect("the matched event payload is archived");
    assert!(archived_forensic_json(payload).contains(&credential_ref));
    let (status, before) = route_request(
        app.clone(),
        axum::http::Method::GET,
        "/plugins/credential_broker/credentials/info",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{before}");
    assert_eq!(before["plugin_id"], "credential_broker");
    assert_eq!(before["store"]["backend"], "disk_override");
    assert_eq!(before["inventory"][0]["credential_ref"], credential_ref);
    assert_eq!(before["inventory"][0]["replay_available"], false);

    let (status, after) = route_request(
        app,
        axum::http::Method::POST,
        "/plugins/credential_broker/credentials/reload",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{after}");
    assert_eq!(after["plugin_id"], "credential_broker");
    assert_eq!(after["store"]["ready"], true);
    assert_eq!(after["store"]["status"], "ready");
    assert_eq!(after["store"]["backend"], "disk_override");
    assert_eq!(after["store"]["last_hydrated_count"], 1);
    assert!(after["store"]["last_hydrated_unix_ms"].as_u64().is_some());
    assert_eq!(after["inventory"][0]["credential_ref"], credential_ref);
    assert_eq!(after["inventory"][0]["replay_available"], true);
    let refreshed_reader = state
        .session_db_handle("broker-reload-vm")
        .expect("reload response re-registers session DB reader");
    assert!(
        !Arc::ptr_eq(&stale_reader, &refreshed_reader),
        "credential broker reload must rebuild session DB readers before reporting inventory"
    );
}

#[tokio::test]
async fn credential_broker_plugin_runtime_reports_security_ledger_activity() {
    let _lock = SETTINGS_ENV_LOCK.lock().await;
    let state = make_test_state();
    let app = build_service_router(Arc::clone(&state));
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("sessions").join("broker-vm");
    std::fs::create_dir_all(&session_dir).unwrap();
    insert_fake_instance_with_session_dir(&state, "broker-vm", std::process::id(), session_dir.clone());

    let event_json = r#"{
        "event_type": "http.request",
        "credential_observations": [
            {
                "provider": "google",
                "source": "http.body.response.$.access_token",
                "event_type": "http.request",
                "trace_id": null,
                "context_json": {"domain":"oauth2.googleapis.com"},
                "credential_ref": "credential:blake3:1111111111111111111111111111111111111111111111111111111111111111"
            }
        ],
        "credential_injections": [
            {
                "provider": "google",
                "source": "http.request.header.authorization",
                "event_type": "http.request",
                "trace_id": null,
                "context_json": {"domain":"generativelanguage.googleapis.com"},
                "credential_ref": "credential:blake3:1111111111111111111111111111111111111111111111111111111111111111"
            }
        ]
    }"#;
    let session_db = session_dir.join("session.db");
    let writer = capsem_logger::DbWriter::open(&session_db, 16).unwrap();
    writer
        .write(capsem_logger::WriteOp::SecurityRuleEvent(
            capsem_logger::SecurityRuleEvent::new(
                1_789_000_123_456,
                "abc123def456",
                "http.request",
                "profiles.rules.default_http",
                r#"{"name":"default_http"}"#,
                event_json,
            ),
        ))
        .await;
    writer.shutdown_blocking();
    let direct_rows = capsem_logger::DbReader::open(&session_db)
        .unwrap()
        .recent_security_rule_events(10)
        .unwrap();
    assert_eq!(direct_rows.len(), 1);
    // The forensic payload is archive-backed now, so the ledger holds it only
    // if the archive gives it back.
    let payload = capsem_logger::DbHandle::open_external_reader(&session_db)
        .unwrap()
        .read_body(
            "abc123def456",
            "security_rule_events",
            capsem_logger::BodyDirection::Payload,
        )
        .await
        .unwrap()
        .expect("the matched event payload is archived");
    assert!(archived_forensic_json(payload).contains("credential_observations"));
    let (status, list) = route_request(app.clone(), axum::http::Method::GET, "/plugins/list", None).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let broker = list["plugins"]
        .as_array()
        .unwrap()
        .iter()
        .find(|plugin| plugin["id"] == "credential_broker")
        .expect("credential broker plugin is listed");
    assert_eq!(
        broker["runtime"]["event_count"], 0,
        "plugin list is a hot config route and must not hydrate runtime ledgers"
    );

    let (status, broker) = route_request(app, axum::http::Method::GET, "/plugins/credential_broker/info", None).await;
    assert_eq!(status, StatusCode::OK, "{broker}");
    assert_eq!(broker["runtime"]["event_count"], 2);
    assert_eq!(broker["runtime"]["rewrite_count"], 1);
    assert_eq!(
        broker["runtime"]["brokered_credentials"][0]["credential_ref"],
        "credential:blake3:1111111111111111111111111111111111111111111111111111111111111111"
    );
    assert_eq!(broker["runtime"]["brokered_credentials"][0]["provider"], "google");
    assert_eq!(broker["runtime"]["brokered_credentials"][0]["observed_count"], 1);
    assert_eq!(broker["runtime"]["brokered_credentials"][0]["injected_count"], 1);
    assert_eq!(
        broker["runtime"]["brokered_credentials"][0]["replay_available"], false,
        "security event evidence alone must not imply the broker can replay the credential"
    );
}

#[tokio::test]
async fn plugin_runtime_reports_execution_latency_from_security_ledger_payloads() {
    let _lock = SETTINGS_ENV_LOCK.lock().await;
    let state = make_test_state();
    let app = build_service_router(Arc::clone(&state));
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("sessions").join("plugin-vm");
    std::fs::create_dir_all(&session_dir).unwrap();
    insert_fake_instance_with_session_dir(&state, "plugin-vm", std::process::id(), session_dir.clone());

    let event_json = r#"{
        "event_type": "http.request",
        "plugin_executions": [
            {
                "plugin_id": "credential_broker",
                "stage": "preprocess",
                "applied": false,
                "duration_us": 13
            },
            {
                "plugin_id": "log_sanitizer",
                "stage": "logging",
                "applied": true,
                "duration_us": 77
            },
            {
                "plugin_id": "dummy_post_allow",
                "stage": "postprocess",
                "applied": true,
                "duration_us": 31
            }
        ],
        "detections": [
            {
                "source": "plugin",
                "detection_level": "informational",
                "rule_id": null,
                "plugin_id": "log_sanitizer",
                "action": null,
                "plugin_mode": "rewrite",
                "reason": null
            },
            {
                "source": "plugin",
                "detection_level": "low",
                "rule_id": null,
                "plugin_id": "dummy_post_allow",
                "action": null,
                "plugin_mode": "allow",
                "reason": null
            }
        ]
    }"#;
    let writer = capsem_logger::DbWriter::open(&session_dir.join("session.db"), 16).unwrap();
    for rule_id in ["profiles.rules.default_http", "profiles.rules.ai_google"] {
        writer
            .write(capsem_logger::WriteOp::SecurityRuleEvent(
                capsem_logger::SecurityRuleEvent::new(
                    1_789_000_123_456,
                    "abc123def456",
                    "http.request",
                    rule_id,
                    r#"{"name":"default_http"}"#,
                    event_json,
                ),
            ))
            .await;
    }
    writer.shutdown_blocking();
    let (status, list) = route_request(app.clone(), axum::http::Method::GET, "/plugins/list", None).await;
    assert_eq!(status, StatusCode::OK, "{list}");

    let sanitizer = list["plugins"]
        .as_array()
        .unwrap()
        .iter()
        .find(|plugin| plugin["id"] == "log_sanitizer")
        .expect("log sanitizer plugin is listed");
    assert_eq!(
        sanitizer["runtime"]["execution_count"], 0,
        "plugin list is a hot config route and must not hydrate runtime DB scans"
    );

    let (status, sanitizer_detail) = route_request(
        app.clone(),
        axum::http::Method::GET,
        "/plugins/log_sanitizer/info",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{sanitizer_detail}");
    assert_eq!(
        sanitizer_detail["runtime"]["execution_count"], 1,
        "multiple rule rows for one security event must not double-count one plugin execution"
    );
    assert_eq!(sanitizer_detail["runtime"]["applied_count"], 1);
    assert_eq!(sanitizer_detail["runtime"]["skipped_count"], 0);
    assert_eq!(sanitizer_detail["runtime"]["detection_count"], 1);
    assert_eq!(sanitizer_detail["runtime"]["total_duration_us"], 77);
    assert_eq!(sanitizer_detail["runtime"]["max_duration_us"], 77);

    let (status, dummy_post) = route_request(
        app.clone(),
        axum::http::Method::GET,
        "/plugins/dummy_post_allow/info",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{dummy_post}");
    assert_eq!(
        dummy_post["runtime"]["execution_count"], 1,
        "postprocess plugin executions must hydrate from the same security ledger payloads"
    );
    assert_eq!(dummy_post["runtime"]["applied_count"], 1);
    assert_eq!(dummy_post["runtime"]["skipped_count"], 0);
    assert_eq!(dummy_post["runtime"]["detection_count"], 1);
    assert_eq!(dummy_post["runtime"]["total_duration_us"], 31);
    assert_eq!(dummy_post["runtime"]["max_duration_us"], 31);

    let (status, broker) = route_request(app, axum::http::Method::GET, "/plugins/credential_broker/info", None).await;
    assert_eq!(status, StatusCode::OK, "{broker}");
    assert_eq!(broker["runtime"]["execution_count"], 1);
    assert_eq!(broker["runtime"]["applied_count"], 0);
    assert_eq!(broker["runtime"]["skipped_count"], 1);
    assert_eq!(broker["runtime"]["total_duration_us"], 13);
}

#[tokio::test]
async fn plugin_edits_through_the_router_change_the_listed_config() {
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let (_env_guard, settings_path, _) = install_empty_settings_env(&dir);
    let app = build_service_router(make_test_state());

    let (status, list) = route_request(app.clone(), axum::http::Method::GET, "/plugins/list", None).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let dummy_pre = list["plugins"]
        .as_array()
        .unwrap()
        .iter()
        .find(|plugin| plugin["id"] == "dummy_pre_eicar")
        .expect("dummy_pre_eicar listed")
        .clone();
    assert_eq!(dummy_pre["config"]["mode"], "disable");
    assert_eq!(dummy_pre["overridden"], false);

    let (status, edited) = route_request(
        app.clone(),
        axum::http::Method::PATCH,
        "/plugins/dummy_pre_eicar/edit",
        Some(json!({ "mode": "block", "detection_level": "critical" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{edited}");
    assert_eq!(edited["config"]["mode"], "block");
    assert_eq!(edited["config"]["detection_level"], "critical");
    assert_eq!(edited["overridden"], true);

    let (status, list) = route_request(app.clone(), axum::http::Method::GET, "/plugins/list", None).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let listed = list["plugins"]
        .as_array()
        .unwrap()
        .iter()
        .find(|plugin| plugin["id"] == "dummy_pre_eicar")
        .expect("edited plugin remains listed");
    assert_eq!(listed["config"]["mode"], "block");
    assert_eq!(listed["config"]["detection_level"], "critical");
    assert_eq!(listed["overridden"], true);

    let (status, info) = route_request(
        app.clone(),
        axum::http::Method::GET,
        "/plugins/dummy_pre_eicar/info",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{info}");
    assert_eq!(info["config"]["mode"], "block");
    let settings = std::fs::read_to_string(&settings_path).unwrap();
    assert!(settings.contains("[plugins.dummy_pre_eicar]"), "{settings}");

    for uri in ["/plugins/teleporter/info", "/plugins/teleporter/edit"] {
        let method = if uri.ends_with("info") {
            axum::http::Method::GET
        } else {
            axum::http::Method::PATCH
        };
        let (status, _) = route_request(app.clone(), method, uri, Some(json!({ "mode": "block" }))).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
    }
}

#[test]
fn plugin_edit_payloads_fail_closed() {
    for payload in [
        json!({ "mode": "teleport" }),
        json!({ "detection_level": "panic" }),
        json!({ "mode": "rewrite", "credential_ref": "sk-leak" }),
    ] {
        assert!(
            serde_json::from_value::<crate::plugin_routes::PluginUpdate>(payload.clone()).is_err(),
            "{payload}"
        );
    }
}

#[tokio::test]
async fn credential_broker_detail_route_exposes_inventory_and_grant_surface() {
    // The store status and the plugin policy read process-wide settings that
    // neighbouring tests swap under this lock; reading them unlocked raced.
    let _lock = SETTINGS_ENV_LOCK.lock().await;
    let app = build_service_router(make_test_state());

    let (status, detail) = route_request(
        app,
        axum::http::Method::GET,
        "/plugins/credential_broker/credentials/info",
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(detail["plugin_id"], "credential_broker");
    assert!(detail.get("scope").is_none(), "{detail}");
    assert_eq!(
        detail["store"]["backend"],
        json!(capsem_core::credential_broker::credential_store_status().backend)
    );
    assert!(detail["inventory"].as_array().unwrap().is_empty());
    assert_eq!(detail["grants"]["fork_default"], "inherit");
    assert!(
        detail["grants"]["vm_grants"].as_array().unwrap().is_empty(),
        "VM-specific credential grants are explicit overrides, not hidden defaults"
    );
    assert!(detail["corp_constraints"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn service_status_reports_ready_empty_credential_store_without_inventory_counters() {
    let _lock = SETTINGS_ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let _store_guard = EnvVarGuard::set("CAPSEM_CREDENTIAL_STORE_PATH", dir.path().join("credential-store.json"));
    capsem_core::credential_broker::hydrate_credential_runtime_cache_from_durable_store().unwrap();

    let app = build_service_router(make_test_state());
    let (status, body) = route_request(app, axum::http::Method::GET, "/status", None).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["ready"], true);
    assert_eq!(body["components"]["credential_store"]["ready"], true);
    assert_eq!(body["components"]["credential_store"]["status"], "ready");
    assert_eq!(
        body["components"]["credential_store"]["last_error"],
        serde_json::Value::Null
    );
    assert!(
        body["components"]["credential_store"]["cached_count"].is_null(),
        "credential inventory counters belong to the credential broker object, not /status"
    );
}
