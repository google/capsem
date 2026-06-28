//! Tests for `client` (extracted from inline `mod tests`).

use super::*;

struct EnvGuard {
    key: &'static str,
    prev: Option<String>,
}

impl EnvGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let prev = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, prev }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.prev {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

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
    let json = r#"{"id":"vm-1","name":"code-vm1","profile_id":"code","status":"Running","persistent":true,"can_resume":false,"available_actions":["pause","stop","fork","delete"]}"#;
    let resp: ApiResponse<ProvisionResponse> = serde_json::from_str(json).unwrap();
    let result = resp.into_result().unwrap();
    assert_eq!(result.id, "vm-1");
    assert_eq!(result.name, "code-vm1");
    assert_eq!(result.profile_id, "code");
    assert_eq!(result.status, VmLifecycleState::Running);
    assert!(result.persistent);
    assert!(!result.can_resume);
    assert_eq!(
        result.available_actions,
        vec![
            VmAction::Pause,
            VmAction::Stop,
            VmAction::Fork,
            VmAction::Delete
        ]
    );
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
fn update_status_response_parses_service_contract() {
    let json = r#"{
        "checked_at": 1718444400,
        "channel_url": "https://release.capsem.org/health.json",
        "stale": false,
        "binary": {
            "current": "1.3.1782582155",
            "latest": "1.3.1782600000",
            "update_available": true,
            "state": "update_available",
            "compatibility": "compatible"
        },
        "assets": {
            "current": "2026.0627.1",
            "latest": "2026.0628.1",
            "update_available": true,
            "state": "update_available",
            "compatibility": "compatible"
        },
        "profiles": {
            "update_available": false,
            "state": "not_published",
            "compatibility": "not_applicable"
        },
        "images": {
            "update_available": false,
            "state": "not_published",
            "compatibility": "not_applicable"
        }
    }"#;

    let status: UpdateStatusResponse = serde_json::from_str(json).unwrap();

    assert_eq!(status.checked_at, Some(1718444400));
    assert_eq!(
        status.channel_url.as_deref(),
        Some("https://release.capsem.org/health.json")
    );
    assert!(!status.stale);
    assert_eq!(status.binary.state, UpdateTrackState::UpdateAvailable);
    assert_eq!(
        status.binary.compatibility,
        UpdateCompatibilityState::Compatible
    );
    assert_eq!(status.assets.current.as_deref(), Some("2026.0627.1"));
    assert_eq!(status.profiles.state, UpdateTrackState::NotPublished);
    assert_eq!(
        status.images.compatibility,
        UpdateCompatibilityState::NotApplicable
    );
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
        profile_id: "code".into(),
        ram_mb: 4096,
        cpus: 4,
        persistent: true,
        env: None,
        from: None,
    };
    let json = serde_json::to_string(&req).unwrap();
    let req2: ProvisionRequest = serde_json::from_str(&json).unwrap();
    assert_eq!(req2.name, Some("test".into()));
    assert_eq!(req2.profile_id, "code");
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
        profile_id: "code".into(),
        ram_mb: 2048,
        cpus: 2,
        persistent: true,
        env: Some(env),
        from: None,
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
        profile_id: "code".into(),
        ram_mb: 2048,
        cpus: 2,
        persistent: false,
        env: None,
        from: None,
    };
    let json = serde_json::to_string(&req).unwrap();
    assert!(!json.contains("env"));
}

#[test]
fn provision_request_with_from() {
    let req = ProvisionRequest {
        name: None,
        profile_id: "code".into(),
        ram_mb: 2048,
        cpus: 2,
        persistent: false,
        env: None,
        from: Some("my-sandbox".into()),
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
        profile_id: "code".into(),
        ram_mb: 2048,
        cpus: 2,
        persistent: false,
        env: None,
        from: None,
    };
    let json = serde_json::to_string(&req).unwrap();
    assert!(!json.contains("from"));
}

#[test]
fn list_response_empty_serde() {
    let resp = ListResponse { sessions: vec![] };
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
                status: VmLifecycleState::Running,
                persistent: false,
                ram_mb: Some(2048),
                cpus: Some(2),
                version: Some("0.16.1".into()),
                forked_from: None,
                description: None,
                created_at: None,
                uptime_secs: Some(3600),
                total_input_tokens: None,
                total_output_tokens: None,
                total_estimated_cost: None,
                total_tool_calls: None,
                total_requests: None,
                allowed_requests: None,
                denied_requests: None,
                total_file_events: None,
                model_call_count: None,
                last_error: None,
                can_resume: false,
                resume_blocked_reason: None,
            },
            SessionInfo {
                id: "mydev".into(),
                name: Some("mydev".into()),
                pid: 0,
                status: VmLifecycleState::Stopped,
                persistent: true,
                ram_mb: Some(4096),
                cpus: Some(4),
                version: None,
                forked_from: None,
                description: None,
                created_at: None,
                uptime_secs: None,
                total_input_tokens: None,
                total_output_tokens: None,
                total_estimated_cost: None,
                total_tool_calls: None,
                total_requests: None,
                allowed_requests: None,
                denied_requests: None,
                total_file_events: None,
                model_call_count: None,
                last_error: None,
                can_resume: true,
                resume_blocked_reason: None,
            },
        ],
    };
    let json = serde_json::to_string(&resp).unwrap();
    let resp2: ListResponse = serde_json::from_str(&json).unwrap();
    assert_eq!(resp2.sessions.len(), 2);
    assert_eq!(resp2.sessions[0].id, "vm-1");
    assert!(!resp2.sessions[0].persistent);
    assert_eq!(resp2.sessions[1].id, "mydev");
    assert!(resp2.sessions[1].persistent);
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
        profile_id: "code".into(),
        timeout_secs: Some(60),
        env: Some(env),
    };
    let json = serde_json::to_string(&req).unwrap();
    let req2: RunRequest = serde_json::from_str(&json).unwrap();
    assert_eq!(req2.command, "echo hi");
    assert_eq!(req2.profile_id, "code");
    assert_eq!(req2.timeout_secs, Some(60));
    assert_eq!(req2.env.unwrap().get("KEY").unwrap(), "val");
}

#[test]
fn run_request_env_omitted_when_none() {
    let req = RunRequest {
        command: "ls".into(),
        profile_id: "code".into(),
        timeout_secs: None,
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

#[test]
fn request_does_not_auto_launch_after_explicit_stop_marker() {
    let _lock = crate::lock_test_env();
    let dir = tempfile::tempdir().unwrap();
    let run_dir = dir.path().join("run");
    std::fs::create_dir_all(&run_dir).unwrap();
    let _run = EnvGuard::set("CAPSEM_RUN_DIR", run_dir.to_str().unwrap());

    std::fs::write(service_install::explicit_stop_marker_path(), b"stopped\n").unwrap();
    let client = UdsClient::new(run_dir.join("missing.sock"), true);
    let err = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(client.get::<serde_json::Value>("/status"))
        .unwrap_err();
    let msg = format!("{err:#}");

    assert!(
        msg.contains("explicitly stopped"),
        "request should respect explicit stop marker, got: {msg}"
    );
    assert!(
        msg.contains("capsem start"),
        "error should name the explicit recovery command, got: {msg}"
    );
}
