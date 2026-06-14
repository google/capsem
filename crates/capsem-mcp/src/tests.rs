//! Tests for `main` (extracted from inline `mod tests`).

use super::*;
use serde_json::json;

// -----------------------------------------------------------------------
// Param serde roundtrips
// -----------------------------------------------------------------------

#[test]
fn create_params_camel_case() {
    let json = json!({"name": "test", "ramMb": 4096, "cpuCount": 4});
    let p: CreateParams = serde_json::from_value(json).unwrap();
    assert_eq!(p.name, Some("test".into()));
    assert_eq!(p.ram_mb, Some(4096));
    assert_eq!(p.cpu_count, Some(4));
}

#[test]
fn create_params_all_optional() {
    let json = json!({});
    let p: CreateParams = serde_json::from_value(json).unwrap();
    assert_eq!(p.name, None);
    assert_eq!(p.ram_mb, None);
    assert_eq!(p.cpu_count, None);
}

#[test]
fn create_params_serializes_camel() {
    let p = CreateParams {
        name: Some("vm".into()),
        ram_mb: Some(2048),
        cpu_count: Some(2),
        version: None,
        env: None,
        from: None,
    };
    let v = serde_json::to_value(&p).unwrap();
    assert!(v.get("ramMb").is_some());
    assert!(v.get("cpuCount").is_some());
    // snake_case keys must NOT appear
    assert!(v.get("ram_mb").is_none());
    assert!(v.get("cpu_count").is_none());
}

#[test]
fn default_profile_id_is_primary_profile() {
    assert_eq!(DEFAULT_PROFILE_ID, "code");
}

#[test]
fn create_body_includes_required_profile_id() {
    let params = CreateParams {
        name: Some("vm".into()),
        ram_mb: Some(2048),
        cpu_count: Some(2),
        version: None,
        env: None,
        from: None,
    };
    let body = build_create_body(&params);
    assert_eq!(body["profile_id"], "code");
}

#[test]
fn run_body_includes_required_profile_id() {
    let params = RunParams {
        command: "echo ok".into(),
        timeout: None,
        env: None,
    };
    let body = build_run_body(&params);
    assert_eq!(body["profile_id"], "code");
}

#[test]
fn exec_params_roundtrip() {
    let json = json!({"id": "vm-1", "command": "echo hi"});
    let p: ExecParams = serde_json::from_value(json).unwrap();
    assert_eq!(p.id, "vm-1");
    assert_eq!(p.command, "echo hi");
    assert_eq!(p.timeout, None);
}

#[test]
fn exec_params_with_timeout() {
    let json = json!({"id": "vm-1", "command": "make build", "timeout": 120});
    let p: ExecParams = serde_json::from_value(json).unwrap();
    assert_eq!(p.timeout, Some(120));
}

#[test]
fn file_read_params_roundtrip() {
    let json = json!({"id": "vm-1", "path": "/tmp/test.txt"});
    let p: FileReadParams = serde_json::from_value(json).unwrap();
    assert_eq!(p.path, "/tmp/test.txt");
}

#[test]
fn file_write_params_roundtrip() {
    let json = json!({"id": "vm-1", "path": "/tmp/test.txt", "content": "data"});
    let p: FileWriteParams = serde_json::from_value(json).unwrap();
    assert_eq!(p.content, "data");
}

#[test]
fn inspect_params_roundtrip() {
    let json = json!({"id": "vm-1", "sql": "SELECT 1"});
    let p: InspectParams = serde_json::from_value(json).unwrap();
    assert_eq!(p.sql, "SELECT 1");
}

#[test]
fn id_params_roundtrip() {
    let json = json!({"id": "my-vm"});
    let p: IdParams = serde_json::from_value(json).unwrap();
    assert_eq!(p.id, "my-vm");
}

#[test]
fn logs_params_with_grep() {
    let json = json!({"id": "vm-1", "grep": "error"});
    let p: LogsParams = serde_json::from_value(json).unwrap();
    assert_eq!(p.grep, Some("error".into()));
}

#[test]
fn logs_params_without_grep() {
    let json = json!({"id": "vm-1"});
    let p: LogsParams = serde_json::from_value(json).unwrap();
    assert_eq!(p.grep, None);
    assert_eq!(p.tail, None);
}

#[test]
fn logs_params_with_tail() {
    let json = json!({"id": "vm-1", "tail": 50});
    let p: LogsParams = serde_json::from_value(json).unwrap();
    assert_eq!(p.tail, Some(50));
}

#[test]
fn logs_params_with_grep_and_tail() {
    let json = json!({"id": "vm-1", "grep": "error", "tail": 20});
    let p: LogsParams = serde_json::from_value(json).unwrap();
    assert_eq!(p.grep, Some("error".into()));
    assert_eq!(p.tail, Some(20));
}

#[test]
fn service_logs_params_with_tail() {
    let json = json!({"tail": 100});
    let p: ServiceLogsParams = serde_json::from_value(json).unwrap();
    assert_eq!(p.tail, Some(100));
}

#[test]
fn service_logs_params_with_grep() {
    let json = json!({"grep": "panic"});
    let p: ServiceLogsParams = serde_json::from_value(json).unwrap();
    assert_eq!(p.grep, Some("panic".into()));
}

#[test]
fn service_logs_params_empty() {
    let json = json!({});
    let p: ServiceLogsParams = serde_json::from_value(json).unwrap();
    assert_eq!(p.grep, None);
}

#[test]
fn logs_params_grep_empty_string() {
    let json = json!({"id": "vm-1", "grep": ""});
    let p: LogsParams = serde_json::from_value(json).unwrap();
    assert_eq!(p.grep, Some("".into()));
}

#[test]
fn logs_params_grep_special_chars() {
    let json = json!({"id": "vm-1", "grep": "[ERROR] (connection)"});
    let p: LogsParams = serde_json::from_value(json).unwrap();
    assert_eq!(p.grep, Some("[ERROR] (connection)".into()));
}

#[test]
fn service_logs_params_grep_special_chars() {
    let json = json!({"grep": "status=500"});
    let p: ServiceLogsParams = serde_json::from_value(json).unwrap();
    assert_eq!(p.grep, Some("status=500".into()));
}

// -----------------------------------------------------------------------
// grep_lines
// -----------------------------------------------------------------------

#[test]
fn tail_lines_basic() {
    let text = "line 1\nline 2\nline 3\nline 4\nline 5";
    assert_eq!(tail_lines(text, 2), "line 4\nline 5");
}

#[test]
fn tail_lines_more_than_available() {
    let text = "line 1\nline 2";
    assert_eq!(tail_lines(text, 10), text);
}

#[test]
fn tail_lines_exact() {
    let text = "line 1\nline 2\nline 3";
    assert_eq!(tail_lines(text, 3), text);
}

#[test]
fn tail_lines_empty() {
    assert_eq!(tail_lines("", 5), "");
}

#[test]
fn tail_log_fields_applies_to_all() {
    let mut val = json!({
        "logs": "a\nb\nc\nd\ne",
        "serial_logs": "1\n2\n3\n4\n5",
        "process_logs": "x\ny\nz",
    });
    tail_log_fields(&mut val, 2);
    assert_eq!(val["logs"], "d\ne");
    assert_eq!(val["serial_logs"], "4\n5");
    assert_eq!(val["process_logs"], "y\nz");
}

// -----------------------------------------------------------------------

#[test]
fn grep_lines_filters_case_insensitive() {
    let text = "INFO starting\nERROR bad thing\nINFO ok\nError another";
    assert_eq!(grep_lines(text, "error"), "ERROR bad thing\nError another");
}

#[test]
fn grep_lines_no_match() {
    let text = "INFO starting\nINFO ok";
    assert_eq!(grep_lines(text, "error"), "");
}

#[test]
fn grep_lines_empty_input() {
    assert_eq!(grep_lines("", "error"), "");
}

#[test]
fn grep_lines_empty_pattern_matches_all() {
    let text = "line one\nline two\nline three";
    assert_eq!(grep_lines(text, ""), text);
}

#[test]
fn grep_lines_single_line_match() {
    assert_eq!(grep_lines("only line", "only"), "only line");
}

#[test]
fn grep_lines_single_line_no_match() {
    assert_eq!(grep_lines("only line", "missing"), "");
}

#[test]
fn grep_lines_all_lines_match() {
    let text = "error one\nerror two\nerror three";
    assert_eq!(grep_lines(text, "error"), text);
}

#[test]
fn grep_lines_mixed_case_pattern() {
    let text = "ERROR here\nerror there\nErrOr everywhere";
    assert_eq!(grep_lines(text, "ErRoR"), text);
}

#[test]
fn grep_lines_special_chars_literal() {
    // grep_lines does substring matching, not regex -- special chars are literal
    let text = "rate is 99.9%\nrate is 100%\nno rate here";
    assert_eq!(grep_lines(text, "99.9%"), "rate is 99.9%");
}

#[test]
fn grep_lines_regex_metacharacters_are_literal() {
    let text = "file.rs:10\nfilexrs:10\nno match";
    // "." should NOT match "x" -- it's substring, not regex
    assert_eq!(grep_lines(text, "file.rs"), "file.rs:10");
}

#[test]
fn grep_lines_brackets_literal() {
    let text = "vec[0] = 1\nvec_0 = 1\nother";
    assert_eq!(grep_lines(text, "[0]"), "vec[0] = 1");
}

#[test]
fn grep_lines_unicode() {
    let text = "normal line\nline with \u{00e9}m\u{00f8}ji\nanother";
    assert_eq!(
        grep_lines(text, "\u{00e9}m\u{00f8}"),
        "line with \u{00e9}m\u{00f8}ji"
    );
}

#[test]
fn grep_lines_preserves_line_order() {
    let text = "c third\na first\nb second";
    assert_eq!(grep_lines(text, ""), "c third\na first\nb second");
}

#[test]
fn grep_lines_trailing_newline() {
    // A trailing newline produces an empty last line -- should not appear in output
    let text = "error here\ninfo there\n";
    assert_eq!(grep_lines(text, "error"), "error here");
}

#[test]
fn grep_lines_whitespace_pattern() {
    let text = "  indented\nnot indented\n\ttabbed";
    assert_eq!(grep_lines(text, "\t"), "\ttabbed");
}

// -----------------------------------------------------------------------
// build_exec_body
// -----------------------------------------------------------------------

#[test]
fn exec_body_default_timeout() {
    let params = ExecParams {
        id: "vm-1".into(),
        command: "ls".into(),
        timeout: None,
    };
    let body = build_exec_body(&params);
    assert_eq!(body["command"], "ls");
    assert_eq!(body["timeout_secs"], 30);
    // id must NOT leak into the body -- it goes in the URL path
    assert!(body.get("id").is_none());
}

#[test]
fn exec_body_custom_timeout() {
    let params = ExecParams {
        id: "vm-1".into(),
        command: "make".into(),
        timeout: Some(120),
    };
    let body = build_exec_body(&params);
    assert_eq!(body["timeout_secs"], 120);
}

#[test]
fn exec_body_zero_timeout() {
    let params = ExecParams {
        id: "vm-1".into(),
        command: "echo".into(),
        timeout: Some(0),
    };
    let body = build_exec_body(&params);
    assert_eq!(body["timeout_secs"], 0);
}

// -----------------------------------------------------------------------
// grep_log_fields
// -----------------------------------------------------------------------

#[test]
fn grep_log_fields_filters_all_log_keys() {
    let mut val = json!({
        "logs": "INFO boot\nERROR crash\nINFO done",
        "serial_logs": "serial: ok\nserial: ERROR fail",
        "process_logs": "proc started\nproc ERROR exit",
    });
    grep_log_fields(&mut val, "error");
    assert_eq!(val["logs"], "ERROR crash");
    assert_eq!(val["serial_logs"], "serial: ERROR fail");
    assert_eq!(val["process_logs"], "proc ERROR exit");
}

#[test]
fn grep_log_fields_missing_optional_keys() {
    // serial_logs and process_logs may be absent
    let mut val = json!({ "logs": "INFO ok\nERROR bad" });
    grep_log_fields(&mut val, "error");
    assert_eq!(val["logs"], "ERROR bad");
    assert!(val.get("serial_logs").is_none());
    assert!(val.get("process_logs").is_none());
}

#[test]
fn grep_log_fields_leaves_non_log_keys() {
    let mut val = json!({
        "logs": "INFO ok\nERROR bad",
        "id": "vm-1",
        "status": "running",
    });
    grep_log_fields(&mut val, "error");
    assert_eq!(val["logs"], "ERROR bad");
    // Non-log keys must be untouched
    assert_eq!(val["id"], "vm-1");
    assert_eq!(val["status"], "running");
}

#[test]
fn grep_log_fields_no_match_empties_strings() {
    let mut val = json!({ "logs": "INFO ok\nDEBUG fine" });
    grep_log_fields(&mut val, "panic");
    assert_eq!(val["logs"], "");
}

// -----------------------------------------------------------------------
// UDS path resolution
// -----------------------------------------------------------------------

#[test]
fn uds_path_override_logic() {
    // Test the resolution logic without touching real env vars.
    // If override is Some, use it. If None, fall back to run_dir/service.sock.
    let resolve = |override_val: Option<&str>, run_dir: &str| -> PathBuf {
        override_val
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(run_dir).join("service.sock"))
    };
    assert_eq!(
        resolve(Some("/tmp/custom.sock"), "/ignored"),
        PathBuf::from("/tmp/custom.sock"),
    );
    assert_eq!(
        resolve(None, "/home/user/.capsem/run"),
        PathBuf::from("/home/user/.capsem/run/service.sock"),
    );
}

// -----------------------------------------------------------------------
// inspect_schema
// -----------------------------------------------------------------------

#[test]
fn inspect_schema_contains_create_table() {
    let schema = capsem_logger::schema::CREATE_SCHEMA;
    assert!(schema.contains("CREATE TABLE"));
    assert!(schema.contains("net_events"));
    assert!(schema.contains("model_calls"));
}

// -----------------------------------------------------------------------
// Tool router
// -----------------------------------------------------------------------

#[test]
fn tool_router_registers_all_tools() {
    let tools = CapsemHandler::tool_router();
    let names: Vec<String> = tools
        .list_all()
        .iter()
        .map(|t| t.name.to_string())
        .collect();
    let expected = [
        "capsem_list",
        "capsem_create",
        "capsem_info",
        "capsem_exec",
        "capsem_read_file",
        "capsem_write_file",
        "capsem_inspect_schema",
        "capsem_inspect",
        "capsem_delete",
        "capsem_stop",
        "capsem_suspend",
        "capsem_resume",
        "capsem_persist",
        "capsem_purge",
        "capsem_run",
        "capsem_vm_logs",
        "capsem_service_logs",
        "capsem_version",
        "capsem_fork",
        "capsem_mcp_servers",
        "capsem_mcp_tools",
        "capsem_mcp_call",
        // Observability sprint additions (T2/T3):
        "capsem_panics",
        "capsem_triage",
        "capsem_host_logs",
        "capsem_timeline",
    ];
    for name in &expected {
        assert!(names.contains(&name.to_string()), "Missing tool: {name}");
    }
    assert_eq!(
        names.len(),
        expected.len(),
        "Extra tools registered: {names:?}"
    );
}

// -----------------------------------------------------------------------
// Handler server info
// -----------------------------------------------------------------------

#[test]
fn server_info_name_and_version() {
    let client = Arc::new(UdsClient::new(PathBuf::from("/dev/null")));
    let handler = CapsemHandler { client };
    let info = handler.get_info();
    assert_eq!(info.server_info.name, "capsem-mcp");
    assert!(!info.server_info.version.is_empty());
}

// -----------------------------------------------------------------------
// Security: path construction safety
// -----------------------------------------------------------------------

#[test]
fn path_construction_with_traversal() {
    // Verify how VM IDs flow into URL paths -- a malicious ID could cause path traversal
    let id = "../../../etc/passwd";
    let path = format!("/vms/{}/exec", id);
    assert_eq!(path, "/vms/../../../etc/passwd/exec");
    // This gets sent as an HTTP path; the service must validate the ID
}

#[test]
fn path_construction_with_empty_id() {
    let id = "";
    let path = format!("/vms/{}/exec", id);
    assert_eq!(path, "/vms//exec");
    // Empty IDs should be rejected by the service
}

#[test]
fn path_construction_with_slashes() {
    let id = "vm/../../secret";
    let path = format!("/vms/{}/info", id);
    assert!(
        path.contains("../"),
        "Path traversal attempt preserved in URL"
    );
}

// -----------------------------------------------------------------------
// Security: parameter edge cases
// -----------------------------------------------------------------------

#[test]
fn exec_params_empty_command() {
    let json = json!({"id": "vm-1", "command": ""});
    let p: ExecParams = serde_json::from_value(json).unwrap();
    assert_eq!(p.command, "");
}

#[test]
fn exec_params_timeout_zero() {
    let json = json!({"id": "vm-1", "command": "echo", "timeout": 0});
    let p: ExecParams = serde_json::from_value(json).unwrap();
    assert_eq!(p.timeout, Some(0));
}

#[test]
fn exec_params_timeout_large() {
    let json = json!({"id": "vm-1", "command": "train", "timeout": 3600});
    let p: ExecParams = serde_json::from_value(json).unwrap();
    assert_eq!(p.timeout, Some(3600));
}

#[test]
fn exec_params_very_long_command() {
    let long_cmd = "a".repeat(100_000);
    let json = json!({"id": "vm-1", "command": long_cmd});
    let p: ExecParams = serde_json::from_value(json).unwrap();
    assert_eq!(p.command.len(), 100_000);
}

#[test]
fn exec_params_shell_metacharacters() {
    let json = json!({"id": "vm-1", "command": "echo $(whoami) | base64; rm -rf /"});
    let p: ExecParams = serde_json::from_value(json).unwrap();
    assert!(p.command.contains("$(whoami)"));
    assert!(p.command.contains("rm -rf"));
}

#[test]
fn file_read_params_path_traversal() {
    let json = json!({"id": "vm-1", "path": "../../etc/shadow"});
    let p: FileReadParams = serde_json::from_value(json).unwrap();
    assert!(p.path.contains(".."));
}

#[test]
fn file_write_params_path_traversal() {
    let json = json!({"id": "vm-1", "path": "/etc/crontab", "content": "* * * * * evil"});
    let p: FileWriteParams = serde_json::from_value(json).unwrap();
    assert_eq!(p.path, "/etc/crontab");
}

#[test]
fn inspect_params_sql_injection() {
    let json = json!({"id": "vm-1", "sql": "SELECT 1; DROP TABLE net_events; --"});
    let p: InspectParams = serde_json::from_value(json).unwrap();
    assert!(p.sql.contains("DROP TABLE"));
    // Backend MUST use read-only connection
}

#[test]
fn create_params_with_env() {
    let json = json!({"name": "test", "env": {"API_KEY": "sk-123", "DEBUG": "true"}});
    let p: CreateParams = serde_json::from_value(json).unwrap();
    let env = p.env.unwrap();
    assert_eq!(env.get("API_KEY").unwrap(), "sk-123");
    assert_eq!(env.get("DEBUG").unwrap(), "true");
}

#[test]
fn create_params_without_env() {
    let json = json!({"name": "test"});
    let p: CreateParams = serde_json::from_value(json).unwrap();
    assert!(p.env.is_none());
}

#[test]
fn create_params_zero_resources() {
    let json = json!({"ramMb": 0, "cpuCount": 0});
    let p: CreateParams = serde_json::from_value(json).unwrap();
    assert_eq!(p.ram_mb, Some(0));
    assert_eq!(p.cpu_count, Some(0));
}

#[test]
fn create_params_huge_resources() {
    let json = json!({"ramMb": u64::MAX, "cpuCount": u32::MAX});
    let p: CreateParams = serde_json::from_value(json).unwrap();
    assert_eq!(p.ram_mb, Some(u64::MAX));
    assert_eq!(p.cpu_count, Some(u32::MAX));
}

#[test]
fn id_params_with_null_bytes() {
    let json = json!({"id": "vm-\0-test"});
    let p: IdParams = serde_json::from_value(json).unwrap();
    assert!(p.id.contains('\0'));
}

// -----------------------------------------------------------------------
// inspect_schema validates
// -----------------------------------------------------------------------

#[test]
fn inspect_schema_has_all_tables() {
    let schema = capsem_logger::schema::CREATE_SCHEMA;
    for table in [
        "net_events",
        "model_calls",
        "tool_calls",
        "tool_responses",
        "mcp_calls",
        "fs_events",
    ] {
        assert!(schema.contains(table), "Missing table in schema: {table}");
    }
    assert!(
        !schema.contains("CREATE TABLE IF NOT EXISTS snapshot_events"),
        "hypervisor snapshot state must not be part of session.db activity"
    );
}

// -----------------------------------------------------------------------
// format_service_response: the common dispatch shape
// -----------------------------------------------------------------------

#[test]
fn format_service_response_ok_pretty_prints() {
    let out = format_service_response(Ok(json!({"id": "vm-1", "status": "running"}))).unwrap();
    assert!(out.contains("\"id\""));
    assert!(out.contains("\"vm-1\""));
    assert!(out.contains('\n'), "pretty print should be multi-line");
}

#[test]
fn format_service_response_ok_with_embedded_error_is_err() {
    let err = format_service_response(Ok(json!({"error": "vm not found"}))).unwrap_err();
    assert_eq!(err, "vm not found");
}

#[test]
fn format_service_response_ok_with_non_string_error_field_is_ok() {
    // If the "error" field isn't a string, it's not the service's error shape; keep as-is.
    let out = format_service_response(Ok(json!({"error": 500, "msg": "fail"}))).unwrap();
    assert!(out.contains("\"error\""));
    assert!(out.contains("500"));
}

#[test]
fn format_service_response_err_returns_message() {
    let err = format_service_response(Err(anyhow::anyhow!("conn reset"))).unwrap_err();
    assert!(err.contains("conn reset"));
}

#[test]
fn format_service_response_null_value_is_ok() {
    let out = format_service_response(Ok(Value::Null)).unwrap();
    assert_eq!(out, "null");
}

#[test]
fn format_service_response_array_value_is_ok() {
    let out = format_service_response(Ok(json!([1, 2, 3]))).unwrap();
    assert!(out.contains('1'));
    assert!(out.contains('2'));
    assert!(out.contains('3'));
}

// -----------------------------------------------------------------------
// build_create_body
// -----------------------------------------------------------------------

#[test]
fn create_body_named_is_persistent() {
    let p = CreateParams {
        name: Some("dev".into()),
        ..Default::default()
    };
    let body = build_create_body(&p);
    assert_eq!(body["name"], "dev");
    assert_eq!(body["persistent"], true);
}

#[test]
fn create_body_unnamed_is_ephemeral() {
    let p = CreateParams::default();
    let body = build_create_body(&p);
    assert_eq!(body["persistent"], false);
    assert!(body["name"].is_null());
}

#[test]
fn create_body_includes_resources_when_present() {
    let p = CreateParams {
        name: Some("dev".into()),
        ram_mb: Some(4096),
        cpu_count: Some(4),
        ..Default::default()
    };
    let body = build_create_body(&p);
    assert_eq!(body["ram_mb"], 4096);
    assert_eq!(body["cpus"], 4);
}

#[test]
fn create_body_omits_resources_when_absent() {
    let p = CreateParams {
        name: Some("dev".into()),
        ..Default::default()
    };
    let body = build_create_body(&p);
    assert!(body.get("ram_mb").is_none());
    assert!(body.get("cpus").is_none());
}

#[test]
fn create_body_includes_env_when_present() {
    let mut env = HashMap::new();
    env.insert("API_KEY".to_string(), "sk-123".to_string());
    let p = CreateParams {
        name: Some("dev".into()),
        env: Some(env),
        ..Default::default()
    };
    let body = build_create_body(&p);
    assert_eq!(body["env"]["API_KEY"], "sk-123");
}

#[test]
fn create_body_includes_from_clone_source() {
    let p = CreateParams {
        name: Some("new".into()),
        from: Some("src-vm".into()),
        ..Default::default()
    };
    let body = build_create_body(&p);
    assert_eq!(body["from"], "src-vm");
}

// -----------------------------------------------------------------------
// build_run_body
// -----------------------------------------------------------------------

#[test]
fn run_body_default_timeout_is_60() {
    let p = RunParams {
        command: "echo".into(),
        timeout: None,
        env: None,
    };
    let body = build_run_body(&p);
    assert_eq!(body["command"], "echo");
    assert_eq!(body["timeout_secs"], 60);
}

#[test]
fn run_body_custom_timeout() {
    let p = RunParams {
        command: "make build".into(),
        timeout: Some(900),
        env: None,
    };
    let body = build_run_body(&p);
    assert_eq!(body["timeout_secs"], 900);
}

#[test]
fn run_body_with_env() {
    let mut env = HashMap::new();
    env.insert("FOO".to_string(), "bar".to_string());
    let p = RunParams {
        command: "env".into(),
        timeout: None,
        env: Some(env),
    };
    let body = build_run_body(&p);
    assert_eq!(body["env"]["FOO"], "bar");
}

#[test]
fn run_body_without_env_omits_key() {
    let p = RunParams {
        command: "env".into(),
        timeout: None,
        env: None,
    };
    let body = build_run_body(&p);
    assert!(body.get("env").is_none());
}

// -----------------------------------------------------------------------
// build_fork_body
// -----------------------------------------------------------------------

#[test]
fn fork_body_with_description() {
    let p = ForkParams {
        id: "vm-1".into(),
        name: "fork-a".into(),
        description: Some("dev copy".into()),
    };
    let body = build_fork_body(&p);
    assert_eq!(body["name"], "fork-a");
    assert_eq!(body["description"], "dev copy");
}

#[test]
fn fork_body_without_description() {
    let p = ForkParams {
        id: "vm-1".into(),
        name: "fork-a".into(),
        description: None,
    };
    let body = build_fork_body(&p);
    assert_eq!(body["name"], "fork-a");
    assert!(body["description"].is_null());
}

// -----------------------------------------------------------------------
// build_persist_body / build_purge_body / build_read_file_body
// -----------------------------------------------------------------------

#[test]
fn persist_body_contains_name() {
    let p = PersistParams {
        id: "vm-1".into(),
        name: "promoted".into(),
    };
    let body = build_persist_body(&p);
    assert_eq!(body["name"], "promoted");
    // id is in URL path, not body
    assert!(body.get("id").is_none());
}

#[test]
fn purge_body_all_defaults_to_false() {
    let p = PurgeParams { all: None };
    let body = build_purge_body(&p);
    assert_eq!(body["all"], false);
}

#[test]
fn purge_body_all_true_preserved() {
    let p = PurgeParams { all: Some(true) };
    let body = build_purge_body(&p);
    assert_eq!(body["all"], true);
}

#[test]
fn read_file_body_contains_path_only() {
    let p = FileReadParams {
        id: "vm-1".into(),
        path: "/etc/hostname".into(),
    };
    let body = build_read_file_body(&p);
    assert_eq!(body["path"], "/etc/hostname");
    assert!(body.get("id").is_none());
}

// -----------------------------------------------------------------------
// resolve_uds_path / resolve_run_dir
// -----------------------------------------------------------------------

#[test]
fn resolve_uds_path_prefers_override() {
    let run_dir = std::path::Path::new("/ignored/run");
    assert_eq!(
        resolve_uds_path(Some("/tmp/custom.sock"), run_dir),
        PathBuf::from("/tmp/custom.sock"),
    );
}

#[test]
fn resolve_uds_path_falls_back_to_run_dir() {
    let run_dir = std::path::Path::new("/home/u/.capsem/run");
    assert_eq!(
        resolve_uds_path(None, run_dir),
        PathBuf::from("/home/u/.capsem/run/service.sock"),
    );
}

#[test]
fn resolve_run_dir_prefers_override() {
    assert_eq!(
        resolve_run_dir("/home/u", Some("/tmp/run")),
        PathBuf::from("/tmp/run"),
    );
}

#[test]
fn resolve_run_dir_default_delegates_to_capsem_core() {
    // The `home` arg is ignored now -- the helper reads CAPSEM_HOME /
    // CAPSEM_RUN_DIR from the process env via capsem-core, so the
    // assertion is that it matches what capsem-core returns.
    assert_eq!(
        resolve_run_dir("/ignored", None),
        capsem_core::paths::capsem_run_dir(),
    );
}

// -----------------------------------------------------------------------
// query_string: URL builder for the 4 tools that push filters into the
// URL (capsem_host_logs / panics / triage / timeline). Bug D: previous
// code did raw format!("k={}&", v) interpolation, so grep="multi word"
// produced a malformed URL ("invalid uri character") and grep="foo&bar"
// silently corrupted to grep=foo + a stray empty param. The helper
// encodes values via percent-encoding, drops None entries, and emits
// canonical "?k1=v1&k2=v2" without trailing "&".
// -----------------------------------------------------------------------

#[test]
fn query_string_empty_when_no_params() {
    let q: Vec<(&str, Option<String>)> = vec![("a", None), ("b", None)];
    assert_eq!(query_string(&q), "");
}

#[test]
fn query_string_single_param_no_trailing_amp() {
    let q = vec![("limit", Some("10".to_string()))];
    assert_eq!(query_string(&q), "?limit=10");
}

#[test]
fn query_string_multiple_params_separated_by_amp() {
    let q = vec![
        ("since", Some("5m".to_string())),
        ("limit", Some("3".to_string())),
    ];
    assert_eq!(query_string(&q), "?since=5m&limit=3");
}

#[test]
fn query_string_skips_none_values() {
    // Only the middle is set -- pre-fix code emitted "?since=&limit=10&"
    // with a trailing "&" plus an empty since= token. Helper drops Nones.
    let q: Vec<(&str, Option<String>)> = vec![
        ("since", None),
        ("limit", Some("10".to_string())),
        ("id", None),
    ];
    assert_eq!(query_string(&q), "?limit=10");
}

#[test]
fn query_string_encodes_space_in_value() {
    // The repro that fired in real MCP usage: grep="capsem-gateway spawned"
    // produced "invalid uri character" because the space wasn't encoded.
    let q = vec![("grep", Some("capsem-gateway spawned".to_string()))];
    assert_eq!(query_string(&q), "?grep=capsem-gateway%20spawned");
}

#[test]
fn query_string_encodes_ampersand_in_value() {
    // The silent-corruption repro: grep="foo&bar" used to land on the
    // server as grep=foo plus a separate bar= param. Encoding makes it
    // a single literal value the server sees as "foo&bar".
    let q = vec![("grep", Some("foo&bar".to_string()))];
    assert_eq!(query_string(&q), "?grep=foo%26bar");
}

#[test]
fn query_string_encodes_other_reserved_chars() {
    // = # + % ? all need encoding inside a query value.
    let q = vec![("grep", Some("k=v#frag+1%done?ok".to_string()))];
    let out = query_string(&q);
    // Spot-check the chars that would otherwise be parsed as separators.
    assert!(out.contains("%3D"), "= must be %3D: {out}");
    assert!(out.contains("%23"), "# must be %23: {out}");
    assert!(out.contains("%2B"), "+ must be %2B: {out}");
    assert!(out.contains("%25"), "% must be %25: {out}");
    assert!(out.contains("%3F"), "? must be %3F: {out}");
}

#[test]
fn query_string_leaves_safe_chars_unencoded() {
    // Letters, digits, and the unreserved set (-._~) round-trip plain.
    let q = vec![("trace_id", Some("abc-123_def.4~5".to_string()))];
    assert_eq!(query_string(&q), "?trace_id=abc-123_def.4~5");
}
