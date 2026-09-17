//! Plugin runtime hydrated from a session's security ledger.
//!
//! These are the routes that do not just list rule matches but parse their
//! forensic payloads -- to count plugin executions and brokered credentials --
//! so they are the ones that read the archive as well as the row.

use super::*;

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
        .read_body("abcd1234ef56", capsem_logger::BodyDirection::Payload)
        .await
        .unwrap()
        .expect("the matched event payload is archived");
    assert!(String::from_utf8(payload.bytes).unwrap().contains(&credential_ref));
    let (status, before) = route_request(
        app.clone(),
        axum::http::Method::GET,
        "/profiles/code/plugins/credential_broker/credentials/info",
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
        "/profiles/code/plugins/credential_broker/credentials/reload",
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
        "credential broker reload must rebuild profile session DB readers before reporting inventory"
    );
}

#[tokio::test]
async fn credential_broker_plugin_runtime_reports_security_ledger_activity() {
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
        .read_body("abc123def456", capsem_logger::BodyDirection::Payload)
        .await
        .unwrap()
        .expect("the matched event payload is archived");
    assert!(String::from_utf8(payload.bytes)
        .unwrap()
        .contains("credential_observations"));
    let (status, list) = route_request(
        app.clone(),
        axum::http::Method::GET,
        "/profiles/code/plugins/list",
        None,
    )
    .await;
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

    let (status, broker) = route_request(
        app,
        axum::http::Method::GET,
        "/profiles/code/plugins/credential_broker/info",
        None,
    )
    .await;
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
    let _env_lock = SETTINGS_ENV_LOCK.lock().await;
    let profile_dir = tempfile::tempdir().unwrap();
    let (config_root, profile) = install_file_asset_profile_fixture(&profile_dir);
    let _profiles_guard = EnvVarGuard::set("CAPSEM_PROFILES_DIR", config_root.join("profiles"));
    let state = make_asset_state(profile_dir.path().join("assets"));
    let app = build_service_router(Arc::clone(&state));
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("sessions").join("plugin-vm");
    std::fs::create_dir_all(&session_dir).unwrap();
    insert_fake_instance_with_session_dir_and_pins(
        &state,
        "plugin-vm",
        std::process::id(),
        session_dir.clone(),
        profile.revision.clone(),
        profile_payload_hash(&profile).unwrap(),
        profile_asset_pins(&profile).unwrap(),
    );

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
    let (status, list) = route_request(
        app.clone(),
        axum::http::Method::GET,
        "/profiles/code/plugins/list",
        None,
    )
    .await;
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
        "/profiles/code/plugins/log_sanitizer/info",
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
        "/profiles/code/plugins/dummy_post_allow/info",
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

    let (status, broker) = route_request(
        app,
        axum::http::Method::GET,
        "/profiles/code/plugins/credential_broker/info",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{broker}");
    assert_eq!(broker["runtime"]["execution_count"], 1);
    assert_eq!(broker["runtime"]["applied_count"], 0);
    assert_eq!(broker["runtime"]["skipped_count"], 1);
    assert_eq!(broker["runtime"]["total_duration_us"], 13);
}
