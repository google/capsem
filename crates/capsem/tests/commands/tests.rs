use super::service::FakeService;
use serde_json::{json, Value};
use std::{process::Output, time::Duration};

async fn command(service: &FakeService, args: &[&str]) -> Output {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("settings.toml"),
        "[settings.\"app.auto_update\"]\nvalue = false\nmodified = \"test\"\n",
    )
    .unwrap();
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_capsem"));
    child
        .env_clear()
        .env("HOME", directory.path())
        .env("CAPSEM_HOME", directory.path())
        .env("CAPSEM_RUN_DIR", directory.path().join("run"))
        .arg("--uds-path")
        .arg(&service.socket)
        .args(args)
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
        child.env("LLVM_PROFILE_FILE", profile);
    }
    tokio::time::timeout(Duration::from_secs(10), child.output())
        .await
        .expect("CLI command exceeded its deadline")
        .unwrap()
}

fn success(output: Output) -> String {
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).unwrap()
}

fn session(persistent: bool) -> Value {
    json!({"id": "vm-canonical", "name": "work", "pid": 17, "status": "Stopped", "persistent": persistent,
        "ram_mb": 12288, "cpus": 4, "version": "0.7.0", "forked_from": "seed", "description": "project",
        "created_at": "2026-10-09T12:00:00Z", "uptime_secs": 3660, "total_input_tokens": 11,
        "total_output_tokens": 19, "total_estimated_cost": 0.12, "total_tool_calls": 3,
        "total_requests": 8, "allowed_requests": 6, "denied_requests": 2, "total_file_events": 5,
        "model_call_count": 2, "last_error": null, "can_resume": false, "resume_blocked_reason": "image is missing"})
}

fn listed(service: &FakeService, persistent: bool) {
    service.route("GET", "/vms/list", 200, json!({"sandboxes": [session(persistent)]}));
}

#[tokio::test]
async fn list_and_info_render_service_state_and_resolve_names_to_ids() {
    let service = FakeService::start();
    listed(&service, true);
    service.route("GET", "/vms/vm-canonical/info", 200, session(true));
    let list = success(command(&service, &["list"]).await);
    assert!(
        list.contains("vm-canonical") && list.contains("work") && list.contains("12 GB"),
        "{list}"
    );
    assert!(list.contains("image is missing"), "{list}");
    assert_eq!(success(command(&service, &["list", "--quiet"]).await), "vm-canonical\n");
    let info = success(command(&service, &["info", "work"]).await);
    for expected in [
        "Session: vm-canonical",
        "Problem: image is missing",
        "RAM:     12 GB",
        "CPUs:    4",
        "Forked:  seed",
        "Desc:    project",
        "Input Tokens:  11",
        "Output Tokens: 19",
        "6 allowed, 2 denied",
    ] {
        assert!(info.contains(expected), "missing {expected}: {info}");
    }
    let info: Value = serde_json::from_str(&success(command(&service, &["info", "work", "--json"]).await)).unwrap();
    assert_eq!(info, session(true));
    assert_eq!(
        service.calls(),
        [
            "GET /vms/list",
            "GET /vms/list",
            "GET /vms/list",
            "GET /vms/vm-canonical/info",
            "GET /vms/list",
            "GET /vms/vm-canonical/info"
        ]
    );
}

#[tokio::test]
async fn lifecycle_commands_use_canonical_routes_and_exact_mutations() {
    let service = FakeService::start();
    listed(&service, true);
    service.route("GET", "/vms/vm-canonical/info", 200, session(true));
    service.route(
        "POST",
        "/vms/vm-canonical/resume",
        200,
        json!({"id": "vm-canonical", "name": "work", "status": "Running", "available_actions": []}),
    );
    for action in ["pause", "stop", "save"] {
        service.route("POST", &format!("/vms/vm-canonical/{action}"), 200, json!({}));
    }
    service.route("DELETE", "/vms/vm-canonical/delete", 200, json!({}));
    service.route(
        "POST",
        "/vms/vm-canonical/fork",
        200,
        json!({"id": "vm-fork", "name": "copy", "size_bytes": 1048576}),
    );
    assert_eq!(success(command(&service, &["resume", "work"]).await), "vm-canonical\n");
    assert_eq!(
        success(command(&service, &["suspend", "work"]).await),
        "Suspending session: work\nSession suspended.\n"
    );
    assert!(success(command(&service, &["persist", "work", "saved"]).await).contains("persistent as \"saved\""));
    assert!(success(command(&service, &["fork", "work", "copy", "--description", "branch"]).await).contains("'copy'"));
    assert_eq!(success(command(&service, &["restart", "work"]).await), "vm-canonical\n");
    assert_eq!(
        success(command(&service, &["delete", "work"]).await),
        "Deleting session: work\nSession deleted.\n"
    );
    let mutations: Vec<_> = service
        .recorded()
        .into_iter()
        .filter(|r| r.method != "GET")
        .map(|r| (r.method.clone(), r.path.clone(), (!r.body.is_empty()).then(|| r.json())))
        .collect();
    assert_eq!(
        mutations,
        vec![
            ("POST".into(), "/vms/vm-canonical/resume".into(), Some(json!({}))),
            ("POST".into(), "/vms/vm-canonical/pause".into(), Some(json!({}))),
            (
                "POST".into(),
                "/vms/vm-canonical/save".into(),
                Some(json!({"name": "saved"}))
            ),
            (
                "POST".into(),
                "/vms/vm-canonical/fork".into(),
                Some(json!({"name": "copy", "description": "branch"}))
            ),
            ("POST".into(), "/vms/vm-canonical/stop".into(), Some(json!({}))),
            ("POST".into(), "/vms/vm-canonical/resume".into(), Some(json!({}))),
            ("DELETE".into(), "/vms/vm-canonical/delete".into(), None),
        ]
    );
}

#[tokio::test]
async fn rejected_or_unknown_sessions_never_dispatch_later_mutations() {
    let service = FakeService::start();
    listed(&service, false);
    service.route("GET", "/vms/vm-canonical/info", 200, session(false));
    let output = command(&service, &["restart", "work"]).await;
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Cannot restart ephemeral session"));
    assert_eq!(service.calls(), ["GET /vms/list", "GET /vms/vm-canonical/info"]);
    let output = command(&service, &["delete", "missing"]).await;
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unknown session name or id"));
    let output = command(&service, &["delete", "../host"]).await;
    assert!(!output.status.success());
    assert_eq!(
        service.calls(),
        ["GET /vms/list", "GET /vms/vm-canonical/info", "GET /vms/list"]
    );
}

#[tokio::test]
async fn retained_failed_logs_use_the_original_id_and_honor_tail() {
    let service = FakeService::start();
    service.route("GET", "/vms/list", 200, json!({"sandboxes": []}));
    service.once(
        "GET",
        "/vms/failed/logs",
        200,
        json!({"logs": "fallback", "process_logs": "first\nsecond\nthird", "serial_logs": "a\nb\nc"}),
    );
    let output = success(command(&service, &["logs", "failed", "--tail", "2"]).await);
    assert_eq!(
        output,
        "--- Process Logs (failed) ---\nsecond\nthird\n--- Serial Logs (failed) ---\nb\nc\n"
    );
    assert_eq!(service.calls(), ["GET /vms/list", "GET /vms/failed/logs"]);
}

#[tokio::test]
async fn history_escapes_search_and_renders_the_actual_rows() {
    let service = FakeService::start();
    listed(&service, true);
    let rows = json!({"commands": [
        {"timestamp":"now", "layer":"exec", "command":"echo hello", "exit_code":0, "duration_ms":2,
         "stdout_preview":"hello", "stderr_preview":null, "details":{"process_name":"shell"}},
        {"timestamp":"later", "layer":"audit", "command":"cat data", "exit_code":null, "duration_ms":null,
         "stdout_preview":null, "stderr_preview":null, "details":{"parent_exe":"/usr/bin/bash", "exe":"/bin/cat"}}
    ], "total":9, "has_more":true});
    service.route("GET", "/vms/vm-canonical/history", 200, rows.clone());
    let output = success(command(&service, &["history", "work", "--tail", "2", "--search", "a & b"]).await);
    assert!(
        output.contains("echo hello") && output.contains("bash>cat") && output.contains("Showing 2 of 9"),
        "{output}"
    );
    let actual: Value = serde_json::from_str(&success(
        command(&service, &["history", "work", "--all", "--json"]).await,
    ))
    .unwrap();
    assert_eq!(actual, rows);
    assert_eq!(
        service.calls(),
        [
            "GET /vms/list",
            "GET /vms/vm-canonical/history?limit=2&layer=all&search=a%20%26%20b",
            "GET /vms/list",
            "GET /vms/vm-canonical/history?limit=100000&layer=all"
        ]
    );
}

#[tokio::test]
async fn mcp_calls_validate_names_and_json_before_dispatch() {
    let service = FakeService::start();
    service.route(
        "POST",
        "/mcp/servers/demo/tools/echo/call",
        200,
        json!({"answer":"hello"}),
    );
    let output: Value = serde_json::from_str(&success(
        command(
            &service,
            &["mcp", "call", "demo__echo", "--args", "{\"text\":\"hello\"}"],
        )
        .await,
    ))
    .unwrap();
    assert_eq!(output, json!({"answer":"hello"}));
    assert_eq!(
        service.find("POST", "/mcp/servers/demo/tools/echo/call")[0].json(),
        json!({"text":"hello"})
    );
    for args in [
        vec!["mcp", "call", "echo"],
        vec!["mcp", "call", "demo__echo", "--args", "bad-json"],
    ] {
        assert!(!command(&service, &args).await.status.success());
    }
    assert_eq!(service.calls(), ["POST /mcp/servers/demo/tools/echo/call"]);
}

#[tokio::test]
async fn unicode_table_cells_truncate_at_utf8_boundaries() {
    let service = FakeService::start();
    listed(&service, true);
    let command_text = "雪".repeat(40);
    service.route(
        "GET",
        "/vms/vm-canonical/history",
        200,
        json!({
            "commands": [{"timestamp":"snowtime", "layer":"exec", "command":command_text,
                "exit_code":0, "duration_ms":2, "stdout_preview":null, "stderr_preview":null, "details":{}}],
            "total":1, "has_more":false
        }),
    );
    let description = format!("a{}", "雪".repeat(30));
    service.route(
        "GET",
        "/mcp/servers/demo/tools/list",
        200,
        json!([{
            "namespaced_name":"demo__echo", "server_name":"demo", "approved":true, "description":description
        }]),
    );
    // Check both old panic paths before asserting their successful output.
    let history = command(&service, &["history", "work"]).await;
    let tools = command(&service, &["mcp", "tools", "--server", "demo"]).await;
    assert!(
        history.status.success() && tools.status.success(),
        "history: {}; tools: {}",
        String::from_utf8_lossy(&history.stderr),
        String::from_utf8_lossy(&tools.stderr)
    );
    let history = success(history);
    let tools = success(tools);
    assert_eq!(
        history
            .lines()
            .find(|line| line.contains("snowtime"))
            .unwrap()
            .split_whitespace()
            .last(),
        Some(format!("{}...", "雪".repeat(25)).as_str())
    );
    assert_eq!(
        tools
            .lines()
            .find(|line| line.starts_with("demo__echo"))
            .unwrap()
            .split_whitespace()
            .last(),
        Some(format!("a{}", "雪".repeat(19)).as_str())
    );
    assert_eq!(
        service.calls(),
        [
            "GET /vms/list",
            "GET /vms/vm-canonical/history?limit=500&layer=all",
            "GET /mcp/servers/demo/tools/list"
        ]
    );
}
