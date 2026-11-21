use super::{command, session, success, FakeService};
use serde_json::{json, Value};

const ID: &str = "00000000-0000-4000-8000-000000000001";
const OBSERVATION: &str = "0123456789ab";

#[tokio::test]
async fn session_info_shows_the_boot_live_and_desired_environment_revisions() {
    let gateway = FakeService::start();
    let mut vm = session(true);
    let boot_revision = "00000000-0000-4000-8000-000000000002";
    let desired_revision = "00000000-0000-4000-8000-000000000003";
    vm["environment"] = json!({
        "boot": {"environment_id": ID, "revision_id": boot_revision},
        "live_policy": {"environment_id": ID, "revision_id": desired_revision},
        "desired": {"environment_id": ID, "revision_id": desired_revision},
        "owner_generation": null, "policy_digest": "reviewed-policy", "pending_restart": true,
    });
    gateway.route("GET", "/vms/list", 200, json!({"sandboxes": [vm]}));
    gateway.route("GET", "/vms/vm-canonical/info", 200, vm);
    let output = success(command(&gateway, &["info", "vm-canonical"]).await);
    for expected in [
        format!("Desired environment: {ID}@{desired_revision}"),
        format!("Boot environment: {ID}@{boot_revision}"),
        format!("Live policy revision: {ID}@{desired_revision}"),
        "Policy digest: reviewed-policy".into(),
        "Workload changes pending restart".into(),
    ] {
        assert!(
            output.lines().any(|line| line == expected),
            "missing {expected}: {output}"
        );
    }
}

fn summary() -> Value {
    json!({
        "request_id": ID, "kind": "environment_change", "environment_id": ID,
        "base": null, "source": null, "requester": null, "actor": "delegated",
        "reason": "change startup", "timestamp_unix_ms": 1, "expires_at_unix_ms": 1000,
        "decision": "pending", "application": "pending", "result": null,
    })
}

fn detail(summary: Value) -> Value {
    json!({
        "summary": summary, "observed_revision": OBSERVATION, "disposition": null,
        "base": null, "targets": [],
        "proposed": {"availability": "complete", "purged_variables": [], "specification": {
            "name": "work", "startup": {"script": "echo first\necho second", "timeout_seconds": 10}
        }},
    })
}

#[tokio::test]
async fn requests_commands_use_the_gateway_and_forward_the_explicit_decision() {
    let gateway = FakeService::start();
    let page = json!({"requests": [summary()], "next_cursor": 9});
    let detail = detail(summary());
    let info_path = format!("/requests/{ID}/info");
    let resolve_path = format!("/requests/{ID}/resolve");
    let receipt = json!({"request_id": ID, "environment_id": ID, "decision": "approved", "application": "pending",
        "result": {"environment_id": ID, "revision_id": ID}, "application_error": null});
    gateway.route("GET", "/requests/list", 200, page.clone());
    gateway.route("GET", &info_path, 200, detail.clone());
    gateway.route("POST", &resolve_path, 200, receipt.clone());

    let output = command(&gateway, &["requests", "list", "--after", "7", "--limit", "1"]).await;
    assert_eq!(serde_json::from_str::<Value>(&success(output)).unwrap(), page);
    let output = command(&gateway, &["requests", "list", "--all"]).await;
    assert_eq!(serde_json::from_str::<Value>(&success(output)).unwrap(), page);
    let output = command(&gateway, &["requests", "info", ID]).await;
    let review: Value = serde_json::from_str(&success(output)).unwrap();
    assert_eq!(review["observed_revision"], detail["observed_revision"]);
    assert_eq!(
        review["proposed"]["specification"]["startup"],
        detail["proposed"]["specification"]["startup"]
    );
    let output = command(
        &gateway,
        &[
            "requests",
            "resolve",
            ID,
            "approve",
            "--observed-revision",
            OBSERVATION,
            "--reason",
            "reviewed the full startup script",
        ],
    )
    .await;
    assert_eq!(serde_json::from_str::<Value>(&success(output)).unwrap(), receipt);
    assert_eq!(
        gateway.calls(),
        [
            "GET /requests/list?after=7&limit=1&pending_only=true".to_string(),
            "GET /requests/list?pending_only=false".to_string(),
            format!("GET {info_path}"),
            format!("POST {resolve_path}"),
        ]
    );
    assert_eq!(
        gateway.find("POST", &resolve_path)[0].json(),
        json!({
            "observed_revision": OBSERVATION, "decision": "approve", "reason": "reviewed the full startup script",
        })
    );
}

#[tokio::test]
async fn requests_commands_refuse_unreviewed_decisions_and_mismatched_or_stale_receipts() {
    let gateway = FakeService::start();
    let mut wrong = summary();
    wrong["request_id"] = json!("00000000-0000-4000-8000-000000000002");
    gateway.route("GET", &format!("/requests/{ID}/info"), 200, detail(wrong));
    let resolve = format!("/requests/{ID}/resolve");
    gateway.route("POST", &resolve, 409, json!({"error": "request observation changed"}));

    let missing = command(
        &gateway,
        &["requests", "resolve", ID, "approve", "--reason", "reviewed"],
    )
    .await;
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("--observed-revision"));
    let malformed = command(
        &gateway,
        &[
            "requests",
            "resolve",
            ID,
            "approve",
            "--observed-revision",
            "wrong",
            "--reason",
            "reviewed",
        ],
    )
    .await;
    assert!(!malformed.status.success());
    assert!(String::from_utf8_lossy(&malformed.stderr).contains("requires the exact observed"));
    assert!(gateway.calls().is_empty());

    let substituted = command(&gateway, &["requests", "info", ID]).await;
    assert!(!substituted.status.success());
    assert!(String::from_utf8_lossy(&substituted.stderr).contains("review request identity changed"));
    let stale = command(
        &gateway,
        &[
            "requests",
            "resolve",
            ID,
            "approve",
            "--observed-revision",
            OBSERVATION,
            "--reason",
            "reviewed",
        ],
    )
    .await;
    assert!(!stale.status.success());
    assert!(String::from_utf8_lossy(&stale.stderr).contains("request observation changed"));
    assert_eq!(gateway.find("POST", &resolve).len(), 1);
}
