use super::*;
use serde_json::json;

// -----------------------------------------------------------------------
// ProvisionRequest / ProvisionResponse
// -----------------------------------------------------------------------

#[test]
fn provision_request_with_name() {
    let json = json!({"name": "my-vm", "profile_id": "code", "ram_mb": 4096, "cpus": 4, "persistent": true});
    let r: ProvisionRequest = serde_json::from_value(json).unwrap();
    assert_eq!(r.name, Some("my-vm".into()));
    assert_eq!(r.profile_id, "code");
    assert_eq!(r.ram_mb, Some(4096));
    assert_eq!(r.cpus, Some(4));
    assert!(r.persistent);
    assert!(r.env.is_none());
}

#[test]
fn provision_request_requires_profile_id() {
    let json = json!({"name": "my-vm", "ram_mb": 4096, "cpus": 4});
    let err = serde_json::from_value::<ProvisionRequest>(json).unwrap_err();
    assert!(err.to_string().contains("profile_id"));
}

#[test]
fn provision_request_ram_cpus_omitted_deserializes_as_none() {
    // Service handler fills these from the selected profile. Callers like
    // the tray's "New Session" do not have to duplicate profile resources.
    let json = json!({"name": "my-vm", "profile_id": "code"});
    let r: ProvisionRequest = serde_json::from_value(json).unwrap();
    assert_eq!(r.ram_mb, None);
    assert_eq!(r.cpus, None);
}

#[test]
fn provision_request_with_env() {
    let json = json!({"profile_id": "code", "ram_mb": 2048, "cpus": 2, "env": {"FOO": "bar", "BAZ": "qux"}});
    let r: ProvisionRequest = serde_json::from_value(json).unwrap();
    let env = r.env.unwrap();
    assert_eq!(env.get("FOO").unwrap(), "bar");
    assert_eq!(env.get("BAZ").unwrap(), "qux");
}

#[test]
fn provision_request_env_omitted() {
    let r = ProvisionRequest {
        name: None,
        profile_id: "code".into(),
        ram_mb: Some(2048),
        cpus: Some(2),
        persistent: false,
        env: None,
        from: None,
    };
    let json = serde_json::to_string(&r).unwrap();
    assert!(!json.contains("env"));
    assert!(!json.contains("from"));
}

#[test]
fn provision_request_without_name() {
    let json = json!({"profile_id": "code", "ram_mb": 2048, "cpus": 2});
    let r: ProvisionRequest = serde_json::from_value(json).unwrap();
    assert_eq!(r.name, None);
    assert!(!r.persistent);
}

#[test]
fn provision_request_with_from() {
    let json = json!({"profile_id": "code", "ram_mb": 2048, "cpus": 2, "from": "my-fork"});
    let r: ProvisionRequest = serde_json::from_value(json).unwrap();
    assert_eq!(r.from.as_deref(), Some("my-fork"));
}

#[test]
fn provision_request_image_alias_deserializes_to_from() {
    let json = json!({"profile_id": "code", "ram_mb": 2048, "cpus": 2, "image": "old-img"});
    let r: ProvisionRequest = serde_json::from_value(json).unwrap();
    assert_eq!(r.from.as_deref(), Some("old-img"));
}

#[test]
fn provision_response_roundtrip() {
    let r = ProvisionResponse {
        id: "vm-123".into(),
        name: "co-work1".into(),
        profile_id: "code".into(),
        status: VmLifecycleState::Running,
        persistent: true,
        can_resume: false,
        available_actions: vec![
            VmAction::Pause,
            VmAction::Stop,
            VmAction::Fork,
            VmAction::Delete,
        ],
        uds_path: Some(std::path::PathBuf::from("/tmp/r/instances/vm-123.sock")),
    };
    let json = serde_json::to_string(&r).unwrap();
    let r2: ProvisionResponse = serde_json::from_str(&json).unwrap();
    assert_eq!(r2.id, "vm-123");
    assert_eq!(r2.name, "co-work1");
    assert_eq!(r2.profile_id, "code");
    assert_eq!(r2.status, VmLifecycleState::Running);
    assert!(r2.persistent);
    assert!(!r2.can_resume);
    assert_eq!(
        r2.available_actions,
        vec![
            VmAction::Pause,
            VmAction::Stop,
            VmAction::Fork,
            VmAction::Delete
        ]
    );
    assert_eq!(
        r2.uds_path.as_deref(),
        Some(std::path::Path::new("/tmp/r/instances/vm-123.sock"))
    );
}

// -----------------------------------------------------------------------
// ListResponse
// -----------------------------------------------------------------------

#[test]
fn list_response_empty() {
    let r = ListResponse { sandboxes: vec![] };
    let json = serde_json::to_string(&r).unwrap();
    let r2: ListResponse = serde_json::from_str(&json).unwrap();
    assert!(r2.sandboxes.is_empty());
}

#[test]
fn list_response_multiple() {
    let r = ListResponse {
        sandboxes: vec![
            {
                let mut s = SandboxInfo::new(
                    "a".into(),
                    "code".into(),
                    100,
                    VmLifecycleState::Running,
                    true,
                );
                s.name = Some("a".into());
                s.ram_mb = Some(2048);
                s.cpus = Some(2);
                s
            },
            SandboxInfo::new(
                "b".into(),
                "code".into(),
                200,
                VmLifecycleState::Running,
                false,
            ),
        ],
    };
    let json = serde_json::to_string(&r).unwrap();
    let r2: ListResponse = serde_json::from_str(&json).unwrap();
    assert_eq!(r2.sandboxes.len(), 2);
    assert_eq!(r2.sandboxes[0].id, "a");
    assert!(r2.sandboxes[0].persistent);
    assert_eq!(r2.sandboxes[1].id, "b");
    assert!(!r2.sandboxes[1].persistent);
}

#[test]
fn sandbox_info_optional_fields_omitted() {
    let s = SandboxInfo::new(
        "x".into(),
        "code".into(),
        1,
        VmLifecycleState::Running,
        false,
    );
    let json = serde_json::to_string(&s).unwrap();
    assert!(!json.contains("ram_mb"));
    assert!(!json.contains("cpus"));
}

#[test]
fn sandbox_info_rejects_unknown_lifecycle_state() {
    let json =
        r#"{"id":"x","profile_id":"code","pid":1,"status":"HalfRestored","persistent":true}"#;
    let err = serde_json::from_str::<SandboxInfo>(json).unwrap_err();
    assert!(err.to_string().contains("unknown variant"));
}

// -----------------------------------------------------------------------
// PersistRequest / PurgeRequest / PurgeResponse
// -----------------------------------------------------------------------

#[test]
fn persist_request_roundtrip() {
    let json = json!({"name": "mydev"});
    let r: PersistRequest = serde_json::from_value(json).unwrap();
    assert_eq!(r.name, "mydev");
}

#[test]
fn purge_request_defaults() {
    let json = json!({});
    let r: PurgeRequest = serde_json::from_value(json).unwrap();
    assert!(!r.all);
}

#[test]
fn purge_request_all() {
    let json = json!({"all": true});
    let r: PurgeRequest = serde_json::from_value(json).unwrap();
    assert!(r.all);
}

#[test]
fn purge_response_roundtrip() {
    let r = PurgeResponse {
        purged: 5,
        persistent_purged: 2,
        ephemeral_purged: 3,
    };
    let json = serde_json::to_string(&r).unwrap();
    let r2: PurgeResponse = serde_json::from_str(&json).unwrap();
    assert_eq!(r2.purged, 5);
    assert_eq!(r2.persistent_purged, 2);
    assert_eq!(r2.ephemeral_purged, 3);
}

// -----------------------------------------------------------------------
// RunRequest
// -----------------------------------------------------------------------

#[test]
fn run_request_defaults() {
    // ram_mb/cpus omitted -> None; handler resolves from the profile.
    let json = json!({"command": "echo hello", "profile_id": "code"});
    let r: RunRequest = serde_json::from_value(json).unwrap();
    assert_eq!(r.command, "echo hello");
    assert_eq!(r.profile_id, "code");
    assert_eq!(r.timeout_secs, None);
    assert_eq!(r.ram_mb, None);
    assert_eq!(r.cpus, None);
}

#[test]
fn run_request_requires_profile_id() {
    let json = json!({"command": "echo hello"});
    let err = serde_json::from_value::<RunRequest>(json).unwrap_err();
    assert!(err.to_string().contains("profile_id"));
}

#[test]
fn run_request_custom() {
    let json = json!({"command": "ls", "profile_id": "code", "timeout_secs": 120, "ram_mb": 4096, "cpus": 4});
    let r: RunRequest = serde_json::from_value(json).unwrap();
    assert_eq!(r.timeout_secs, Some(120));
    assert_eq!(r.ram_mb, Some(4096));
    assert_eq!(r.cpus, Some(4));
}

// -----------------------------------------------------------------------
// ExecRequest / ExecResponse
// -----------------------------------------------------------------------

#[test]
fn exec_request_defaults_to_no_timeout() {
    let json = json!({"command": "echo hi"});
    let r: ExecRequest = serde_json::from_value(json).unwrap();
    assert_eq!(r.command, "echo hi");
    assert_eq!(r.timeout_secs, None);
}

#[test]
fn exec_request_custom_timeout() {
    let json = json!({"command": "sleep 10", "timeout_secs": 5});
    let r: ExecRequest = serde_json::from_value(json).unwrap();
    assert_eq!(r.timeout_secs, Some(5));
}

#[test]
fn exec_response_roundtrip() {
    let r = ExecResponse {
        stdout: "hello\n".into(),
        stderr: "".into(),
        exit_code: 0,
        truncated: false,
    };
    let json = serde_json::to_string(&r).unwrap();
    let r2: ExecResponse = serde_json::from_str(&json).unwrap();
    assert_eq!(r2.stdout, "hello\n");
    assert_eq!(r2.exit_code, 0);
}

// -----------------------------------------------------------------------
// File I/O
// -----------------------------------------------------------------------

#[test]
fn write_file_request_roundtrip() {
    let json = json!({"path": "/tmp/f.txt", "content": "data"});
    let r: WriteFileRequest = serde_json::from_value(json).unwrap();
    assert_eq!(r.path, "/tmp/f.txt");
    assert_eq!(r.content, "data");
}

#[test]
fn read_file_response_roundtrip() {
    let r = ReadFileResponse {
        content: "file contents".into(),
    };
    let json = serde_json::to_string(&r).unwrap();
    let r2: ReadFileResponse = serde_json::from_str(&json).unwrap();
    assert_eq!(r2.content, "file contents");
}

// -----------------------------------------------------------------------
// Logs / Error
// -----------------------------------------------------------------------

#[test]
fn logs_response_roundtrip() {
    let r = LogsResponse {
        logs: "Linux boot...\n".into(),
        serial_logs: None,
        process_logs: None,
    };
    let json = serde_json::to_string(&r).unwrap();
    let r2: LogsResponse = serde_json::from_str(&json).unwrap();
    assert!(r2.logs.contains("Linux"));
}

#[test]
fn error_response_roundtrip() {
    let r = ErrorResponse {
        error: "sandbox not found".into(),
    };
    let json = serde_json::to_string(&r).unwrap();
    let r2: ErrorResponse = serde_json::from_str(&json).unwrap();
    assert!(r2.error.contains("not found"));
}

#[test]
fn exec_response_carries_truncation_to_the_client() {
    let r = ExecResponse {
        stdout: "first 10 MiB".into(),
        stderr: String::new(),
        exit_code: 0,
        truncated: true,
    };

    let json = serde_json::to_string(&r).unwrap();
    let back: ExecResponse = serde_json::from_str(&json).unwrap();

    assert!(
        back.truncated,
        "a capped result must not reach the client looking complete"
    );
}

#[test]
fn exec_response_from_an_older_service_decodes_as_not_truncated() {
    // A client built with the field talking to a service without it must read
    // absence as "complete", never as truncated.
    let back: ExecResponse =
        serde_json::from_str(r#"{"stdout":"ok","stderr":"","exit_code":0}"#).unwrap();

    assert!(!back.truncated);
    assert_eq!(back.stdout, "ok");
}
