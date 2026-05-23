//! Tests for `client` (extracted from inline `mod tests`).

use super::*;

// -- validate_id ----------------------------------------------------------

#[test]
fn validate_id_normal() {
    assert!(validate_id("vm-abc123").is_ok());
}

#[test]
fn validate_id_with_dots_no_traversal() {
    assert!(validate_id("vm.abc.123").is_ok());
}

#[test]
fn validate_id_uuid_style() {
    assert!(validate_id("550e8400-e29b-41d4-a716-446655440000").is_ok());
}

#[test]
fn validate_id_rejects_empty() {
    let err = validate_id("").unwrap_err();
    assert!(err.to_string().contains("cannot be empty"), "{}", err);
}

#[test]
fn validate_id_rejects_slash() {
    assert!(validate_id("../etc/passwd").is_err());
}

#[test]
fn validate_id_rejects_backslash() {
    assert!(validate_id("..\\windows\\system32").is_err());
}

#[test]
fn validate_id_rejects_dotdot() {
    assert!(validate_id("..").is_err());
}

#[test]
fn validate_id_rejects_traversal_in_middle() {
    assert!(validate_id("foo/../bar").is_err());
}

#[test]
fn validate_id_rejects_null_byte() {
    assert!(validate_id("vm\0evil").is_err());
}

#[test]
fn validate_id_rejects_absolute_path() {
    assert!(validate_id("/tmp/evil").is_err());
}

// -- parse_env_vars -------------------------------------------------------

#[test]
fn parse_env_vars_empty() {
    assert_eq!(parse_env_vars(&[]).unwrap(), None);
}

#[test]
fn parse_env_vars_single() {
    let vars = vec!["FOO=bar".to_string()];
    let map = parse_env_vars(&vars).unwrap().unwrap();
    assert_eq!(map.len(), 1);
    assert_eq!(map.get("FOO").unwrap(), "bar");
}

#[test]
fn parse_env_vars_multiple() {
    let vars = vec!["A=1".to_string(), "B=2".to_string()];
    let map = parse_env_vars(&vars).unwrap().unwrap();
    assert_eq!(map.len(), 2);
    assert_eq!(map.get("A").unwrap(), "1");
    assert_eq!(map.get("B").unwrap(), "2");
}

#[test]
fn parse_env_vars_value_with_equals() {
    let vars = vec!["URL=http://host?a=1&b=2".to_string()];
    let map = parse_env_vars(&vars).unwrap().unwrap();
    assert_eq!(map.get("URL").unwrap(), "http://host?a=1&b=2");
}

#[test]
fn parse_env_vars_empty_value() {
    let vars = vec!["EMPTY=".to_string()];
    let map = parse_env_vars(&vars).unwrap().unwrap();
    assert_eq!(map.get("EMPTY").unwrap(), "");
}

#[test]
fn parse_env_vars_missing_equals() {
    let vars = vec!["NOVAL".to_string()];
    let err = parse_env_vars(&vars).unwrap_err();
    assert!(err.to_string().contains("KEY=VALUE"));
}

#[test]
fn parse_env_vars_second_entry_invalid() {
    let vars = vec!["OK=1".to_string(), "BAD".to_string()];
    assert!(parse_env_vars(&vars).is_err());
}

// -- ApiResponse ordering -------------------------------------------------

#[test]
fn api_response_ok_variant() {
    let json = r#"{"id":"vm-1"}"#;
    let resp: ApiResponse<ProvisionResponse> = serde_json::from_str(json).unwrap();
    let result = resp.into_result().unwrap();
    assert_eq!(result.id, "vm-1");
}

#[test]
fn provision_response_preserves_profile_provenance() {
    let json = r#"{
      "id": "vm-1",
      "uds_path": "/tmp/capsem/vm-1.sock",
      "profile_id": "coding",
      "profile_revision": "2026.0520.1",
      "profile_status": "current",
      "profile_pin": {
        "profile_id": "coding",
        "profile_revision": "2026.0520.1",
        "profile_payload_hash": "blake3:profile",
        "package_contract_hash": "blake3:packages",
        "base_assets": {
          "asset_version": "2026.0520.1",
          "arch": "arm64",
          "kernel_hash": "blake3:kernel",
          "initrd_hash": "blake3:initrd",
          "rootfs_hash": "blake3:rootfs",
          "guest_abi": "capsem-guest-v1"
        }
      },
      "asset_health": {
        "ready": true,
        "state": "ready",
        "profile_id": "coding",
        "profile_revision": "2026.0520.1",
        "profile_payload_hash": "blake3:profile",
        "profile_assets": [
          {
            "logical_name": "rootfs.squashfs",
            "hash": "blake3:rootfs",
            "source_url": "https://assets.example/rootfs.squashfs",
            "size": 123,
            "content_type": "application/octet-stream"
          }
        ],
        "version": "2026.0520.1",
        "arch": "arm64",
        "missing": [],
        "retry_count": 0,
        "retryable": false,
        "saved_vm_dependencies": []
      }
    }"#;
    let resp: ApiResponse<ProvisionResponse> = serde_json::from_str(json).unwrap();
    let result = resp.into_result().unwrap();

    assert_eq!(result.profile_id.as_deref(), Some("coding"));
    assert_eq!(result.profile_revision.as_deref(), Some("2026.0520.1"));
    assert_eq!(result.profile_status, Some(SessionProfileStatus::Current));
    let pin = result.profile_pin.unwrap();
    assert_eq!(pin.profile_payload_hash.as_deref(), Some("blake3:profile"));
    assert_eq!(pin.package_contract_hash, "blake3:packages");
    assert_eq!(pin.base_assets.unwrap().rootfs_hash, "blake3:rootfs");
    let health = result.asset_health.unwrap();
    assert_eq!(health.profile_assets[0].logical_name, "rootfs.squashfs");
    assert_eq!(health.profile_assets[0].hash, "blake3:rootfs");
}

#[test]
fn api_response_err_variant() {
    let json = r#"{"error":"sandbox not found"}"#;
    let resp: ApiResponse<ProvisionResponse> = serde_json::from_str(json).unwrap();
    let err = resp.into_result().unwrap_err();
    assert!(err.to_string().contains("sandbox not found"));
}

#[test]
fn api_response_ok_tried_first() {
    // A response with an "error" field alongside valid fields should
    // still parse as Ok if the Ok type matches first.
    #[derive(Serialize, Deserialize, Debug)]
    struct HasError {
        error: String,
        extra: String,
    }
    let json = r#"{"error":"not-really","extra":"data"}"#;
    let resp: ApiResponse<HasError> = serde_json::from_str(json).unwrap();
    // Since Ok is tried first and HasError has both fields, it should match Ok
    match resp {
        ApiResponse::Ok(v) => {
            assert_eq!(v.error, "not-really");
            assert_eq!(v.extra, "data");
        }
        ApiResponse::Err(_) => panic!("should have parsed as Ok"),
    }
}

#[test]
fn api_response_err_only_when_ok_fails() {
    // When the JSON only has "error" and the Ok type needs "id",
    // serde falls through to Err variant.
    let json = r#"{"error":"vm not found"}"#;
    let resp: ApiResponse<ProvisionResponse> = serde_json::from_str(json).unwrap();
    assert!(resp.into_result().is_err());
}

#[test]
fn api_response_empty_error() {
    let json = r#"{"error":""}"#;
    let resp: ApiResponse<ProvisionResponse> = serde_json::from_str(json).unwrap();
    assert!(resp.into_result().is_err());
}

// -- Serde round-trips ----------------------------------------------------

#[test]
fn provision_request_serde() {
    let req = ProvisionRequest {
        name: Some("test".into()),
        ram_mb: 4096,
        cpus: 4,
        persistent: true,
        env: None,
        from: None,
        profile_id: None,
        profile_revision: None,
    };
    let json = serde_json::to_string(&req).unwrap();
    let req2: ProvisionRequest = serde_json::from_str(&json).unwrap();
    assert_eq!(req2.name, Some("test".into()));
    assert_eq!(req2.ram_mb, 4096);
    assert!(req2.persistent);
    assert!(req2.env.is_none());
}

#[test]
fn provision_request_with_env() {
    let mut env = HashMap::new();
    env.insert("FOO".into(), "bar".into());
    let req = ProvisionRequest {
        name: Some("test".into()),
        ram_mb: 2048,
        cpus: 2,
        persistent: true,
        env: Some(env),
        from: None,
        profile_id: None,
        profile_revision: None,
    };
    let json = serde_json::to_string(&req).unwrap();
    assert!(json.contains("FOO"));
    let req2: ProvisionRequest = serde_json::from_str(&json).unwrap();
    assert_eq!(req2.env.as_ref().unwrap().get("FOO").unwrap(), "bar");
}

#[test]
fn provision_request_env_omitted_when_none() {
    let req = ProvisionRequest {
        name: None,
        ram_mb: 2048,
        cpus: 2,
        persistent: false,
        env: None,
        from: None,
        profile_id: None,
        profile_revision: None,
    };
    let json = serde_json::to_string(&req).unwrap();
    assert!(!json.contains("env"));
}

#[test]
fn provision_request_with_from() {
    let req = ProvisionRequest {
        name: None,
        ram_mb: 2048,
        cpus: 2,
        persistent: false,
        env: None,
        from: Some("my-sandbox".into()),
        profile_id: None,
        profile_revision: None,
    };
    let json = serde_json::to_string(&req).unwrap();
    assert!(json.contains("my-sandbox"));
    let req2: ProvisionRequest = serde_json::from_str(&json).unwrap();
    assert_eq!(req2.from, Some("my-sandbox".into()));
}

#[test]
fn provision_request_from_omitted_when_none() {
    let req = ProvisionRequest {
        name: None,
        ram_mb: 2048,
        cpus: 2,
        persistent: false,
        env: None,
        from: None,
        profile_id: None,
        profile_revision: None,
    };
    let json = serde_json::to_string(&req).unwrap();
    assert!(!json.contains("from"));
}

#[test]
fn list_response_empty_serde() {
    let resp = ListResponse {
        sessions: vec![],
        asset_health: None,
    };
    let json = serde_json::to_string(&resp).unwrap();
    // Wire format uses "sandboxes" key
    assert!(json.contains("sandboxes"));
    let resp2: ListResponse = serde_json::from_str(&json).unwrap();
    assert!(resp2.sessions.is_empty());
}

#[test]
fn list_response_with_entries() {
    let resp = ListResponse {
        sessions: vec![
            SessionInfo {
                id: "vm-1".into(),
                name: None,
                pid: 100,
                status: "Running".into(),
                persistent: false,
                ram_mb: Some(2048),
                cpus: Some(2),
                version: Some("0.16.1".into()),
                base_assets: Some(SavedVmBaseAssets {
                    asset_version: "2026.0520.1".into(),
                    arch: "arm64".into(),
                    kernel_hash: "blake3:kernel".into(),
                    initrd_hash: "blake3:initrd".into(),
                    rootfs_hash: "blake3:rootfs".into(),
                    guest_abi: None,
                }),
                profile_pin: Some(SavedVmProfilePin {
                    profile_id: "everyday-work".into(),
                    profile_revision: Some("2026.0520.2".into()),
                    profile_payload_hash: Some("blake3:profile".into()),
                    package_contract_hash: "blake3:packages".into(),
                    base_assets: None,
                }),
                forked_from: None,
                description: None,
                profile_id: Some("everyday-work".into()),
                profile_revision: Some("2026.0520.2".into()),
                profile_status: Some(SessionProfileStatus::Current),
                created_at: None,
                uptime_secs: Some(3600),
                total_input_tokens: None,
                total_output_tokens: None,
                total_estimated_cost: None,
                total_tool_calls: None,
                total_mcp_calls: None,
                total_requests: None,
                allowed_requests: None,
                denied_requests: None,
                total_file_events: None,
                model_call_count: None,
                last_error: None,
            },
            SessionInfo {
                id: "mydev".into(),
                name: Some("mydev".into()),
                pid: 0,
                status: "Stopped".into(),
                persistent: true,
                ram_mb: Some(4096),
                cpus: Some(4),
                version: None,
                base_assets: None,
                profile_pin: None,
                forked_from: None,
                description: None,
                profile_id: None,
                profile_revision: None,
                profile_status: Some(SessionProfileStatus::Corrupted),
                created_at: None,
                uptime_secs: None,
                total_input_tokens: None,
                total_output_tokens: None,
                total_estimated_cost: None,
                total_tool_calls: None,
                total_mcp_calls: None,
                total_requests: None,
                allowed_requests: None,
                denied_requests: None,
                total_file_events: None,
                model_call_count: None,
                last_error: None,
            },
        ],
        asset_health: None,
    };
    let json = serde_json::to_string(&resp).unwrap();
    let resp2: ListResponse = serde_json::from_str(&json).unwrap();
    assert_eq!(resp2.sessions.len(), 2);
    assert_eq!(resp2.sessions[0].id, "vm-1");
    assert!(!resp2.sessions[0].persistent);
    assert_eq!(
        resp2.sessions[0].profile_id.as_deref(),
        Some("everyday-work")
    );
    assert_eq!(
        resp2.sessions[0].profile_revision.as_deref(),
        Some("2026.0520.2")
    );
    assert_eq!(
        resp2.sessions[0].profile_status,
        Some(SessionProfileStatus::Current)
    );
    let pin = resp2.sessions[0].profile_pin.as_ref().unwrap();
    assert_eq!(pin.profile_payload_hash.as_deref(), Some("blake3:profile"));
    assert_eq!(pin.package_contract_hash, "blake3:packages");
    assert_eq!(
        resp2.sessions[0]
            .base_assets
            .as_ref()
            .map(|assets| assets.rootfs_hash.as_str()),
        Some("blake3:rootfs")
    );
    assert_eq!(resp2.sessions[1].id, "mydev");
    assert!(resp2.sessions[1].persistent);
    assert_eq!(
        resp2.sessions[1].profile_status,
        Some(SessionProfileStatus::Corrupted)
    );
}

#[test]
fn list_response_as_api_response() {
    // The List endpoint should use ApiResponse wrapping
    let json = r#"{"sandboxes":[]}"#;
    let resp: ApiResponse<ListResponse> = serde_json::from_str(json).unwrap();
    let list = resp.into_result().unwrap();
    assert!(list.sessions.is_empty());
}

#[test]
fn list_response_error_as_api_response() {
    let json = r#"{"error":"service unavailable"}"#;
    let resp: ApiResponse<ListResponse> = serde_json::from_str(json).unwrap();
    let err = resp.into_result().unwrap_err();
    assert!(err.to_string().contains("service unavailable"));
}

#[test]
fn exec_request_serde() {
    let req = ExecRequest {
        command: "ls -la".into(),
        timeout_secs: Some(30),
    };
    let json = serde_json::to_string(&req).unwrap();
    let req2: ExecRequest = serde_json::from_str(&json).unwrap();
    assert_eq!(req2.command, "ls -la");
    assert_eq!(req2.timeout_secs, Some(30));
}

#[test]
fn exec_response_serde() {
    let resp = ExecResponse {
        stdout: "hello\n".into(),
        stderr: "".into(),
        exit_code: 0,
    };
    let json = serde_json::to_string(&resp).unwrap();
    let resp2: ExecResponse = serde_json::from_str(&json).unwrap();
    assert_eq!(resp2.stdout, "hello\n");
    assert_eq!(resp2.exit_code, 0);
}

#[test]
fn exec_response_nonzero_exit() {
    let resp = ExecResponse {
        stdout: "".into(),
        stderr: "not found\n".into(),
        exit_code: 127,
    };
    let json = serde_json::to_string(&resp).unwrap();
    let resp2: ExecResponse = serde_json::from_str(&json).unwrap();
    assert_eq!(resp2.exit_code, 127);
    assert_eq!(resp2.stderr, "not found\n");
}

#[test]
fn exec_response_negative_exit_code() {
    let resp = ExecResponse {
        stdout: "".into(),
        stderr: "killed".into(),
        exit_code: -1,
    };
    let json = serde_json::to_string(&resp).unwrap();
    let resp2: ExecResponse = serde_json::from_str(&json).unwrap();
    assert_eq!(resp2.exit_code, -1);
}

#[test]
fn exec_response_signal_exit_code() {
    // SIGKILL = 137 in Docker-style convention
    let resp = ExecResponse {
        stdout: "".into(),
        stderr: "".into(),
        exit_code: 137,
    };
    assert_eq!(resp.exit_code, 137);
}

#[test]
fn fork_request_serde() {
    let req = ForkRequest {
        name: "my-img".into(),
        description: Some("test image".into()),
    };
    let json = serde_json::to_string(&req).unwrap();
    let req2: ForkRequest = serde_json::from_str(&json).unwrap();
    assert_eq!(req2.name, "my-img");
    assert_eq!(req2.description, Some("test image".into()));
}

#[test]
fn fork_request_description_omitted_when_none() {
    let req = ForkRequest {
        name: "img".into(),
        description: None,
    };
    let json = serde_json::to_string(&req).unwrap();
    assert!(!json.contains("description"));
}

#[test]
fn purge_response_serde() {
    let resp = PurgeResponse {
        purged: 5,
        persistent_purged: 2,
        ephemeral_purged: 3,
    };
    let json = serde_json::to_string(&resp).unwrap();
    let resp2: PurgeResponse = serde_json::from_str(&json).unwrap();
    assert_eq!(resp2.purged, 5);
    assert_eq!(resp2.persistent_purged, 2);
    assert_eq!(resp2.ephemeral_purged, 3);
}

#[test]
fn run_request_serde() {
    let mut env = HashMap::new();
    env.insert("KEY".into(), "val".into());
    let req = RunRequest {
        command: "echo hi".into(),
        timeout_secs: Some(60),
        profile_id: Some("coding".into()),
        profile_revision: Some("2026.0520.1".into()),
        env: Some(env),
    };
    let json = serde_json::to_string(&req).unwrap();
    let req2: RunRequest = serde_json::from_str(&json).unwrap();
    assert_eq!(req2.command, "echo hi");
    assert_eq!(req2.timeout_secs, Some(60));
    assert_eq!(req2.profile_id.as_deref(), Some("coding"));
    assert_eq!(req2.profile_revision.as_deref(), Some("2026.0520.1"));
    assert_eq!(req2.env.unwrap().get("KEY").unwrap(), "val");
}

#[test]
fn run_request_env_omitted_when_none() {
    let req = RunRequest {
        command: "ls".into(),
        timeout_secs: None,
        profile_id: None,
        profile_revision: None,
        env: None,
    };
    let json = serde_json::to_string(&req).unwrap();
    assert!(!json.contains("timeout_secs"));
    assert!(!json.contains("env"));
}

#[test]
fn logs_response_serde() {
    let resp = LogsResponse {
        logs: "boot log".into(),
        serial_logs: Some("serial output".into()),
        process_logs: None,
        security_logs: None,
    };
    let json = serde_json::to_string(&resp).unwrap();
    let resp2: LogsResponse = serde_json::from_str(&json).unwrap();
    assert_eq!(resp2.logs, "boot log");
    assert_eq!(resp2.serial_logs, Some("serial output".into()));
    assert!(resp2.process_logs.is_none());
}

#[test]
fn session_info_defaults() {
    // Missing optional fields should deserialize with defaults
    let json = r#"{"id":"vm-1","pid":0,"status":"Running"}"#;
    let info: SessionInfo = serde_json::from_str(json).unwrap();
    assert_eq!(info.id, "vm-1");
    assert!(!info.persistent);
    assert!(info.ram_mb.is_none());
    assert!(info.cpus.is_none());
    assert!(info.version.is_none());
    assert!(info.name.is_none());
    assert!(info.created_at.is_none());
    assert!(info.uptime_secs.is_none());
    assert!(info.total_input_tokens.is_none());
    assert!(info.total_estimated_cost.is_none());
}

// -- connect_with_timeout : ConnectMode contract -------------------------
//
// Regression guards for the `capsem doctor` "Service manager started
// capsem but socket not ready" bug. FailFast must exit immediately when
// the socket doesn't exist; AwaitStartup must wait inside the 5s poll
// budget for a just-starting service to bind.

#[tokio::test]
async fn connect_fail_fast_errors_immediately_on_missing_socket() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("ghost.sock");
    let client = UdsClient::new(sock.clone(), false);

    let start = std::time::Instant::now();
    let err = client
        .connect_with_timeout(ConnectMode::FailFast)
        .await
        .unwrap_err();
    let elapsed = start.elapsed();

    assert!(
        elapsed < std::time::Duration::from_millis(500),
        "FailFast should short-circuit, not wait the poll budget (took {elapsed:?})"
    );
    let msg = format!("{err:#}");
    assert!(
        msg.contains("socket") || msg.contains(&*sock.display().to_string()),
        "error should mention the socket or path, got {msg}"
    );
}

#[tokio::test]
async fn connect_await_startup_waits_for_late_binder() {
    use tokio::net::UnixListener;

    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("late.sock");
    let sock_for_bind = sock.clone();

    // Bind AFTER a delay -- simulates a service that was just
    // started and hasn't yet called UnixListener::bind.
    let binder = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        UnixListener::bind(&sock_for_bind).unwrap()
    });

    let client = UdsClient::new(sock.clone(), false);
    let stream = client
        .connect_with_timeout(ConnectMode::AwaitStartup)
        .await
        .expect("AwaitStartup must see the late bind within the 5s budget");
    drop(stream);

    // Keep the listener alive until after the connect returned
    // (otherwise the Drop could race with accept).
    drop(binder.await.unwrap());
}

#[tokio::test]
async fn connect_await_startup_eventually_times_out() {
    // Nothing ever binds -- AwaitStartup must still return a
    // timeout error, not hang. Use a short override timeout so
    // the test completes in under a second.
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("never.sock");
    let client = UdsClient::new(sock.clone(), false);

    let start = std::time::Instant::now();
    let err = client
        .connect_with_timeout_for_test(
            ConnectMode::AwaitStartup,
            std::time::Duration::from_millis(300),
        )
        .await
        .unwrap_err();
    let elapsed = start.elapsed();

    assert!(
        elapsed >= std::time::Duration::from_millis(250),
        "should have polled until ~timeout, got {elapsed:?}"
    );
    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "should not exceed budget by much, got {elapsed:?}"
    );
    let msg = format!("{err:#}");
    assert!(
        msg.contains("timed out") || msg.contains("timeout"),
        "expected timeout error, got: {msg}"
    );
}
