//! `GET /vms/{id}/history` pages in SQL: it reads one page of rows, not the
//! ledger.

use std::time::{Duration, SystemTime};

use super::*;

const EXECS: u64 = 30;
const AUDITS: u64 = 30;

fn at(secs: u64) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000 + secs)
}

/// A ledger with interleaved exec and audit rows, plus two exec rows whose
/// commands differ only where a LIKE pattern would see wildcards.
async fn history_app() -> (axum::Router, tempfile::TempDir, Vec<(String, String)>) {
    let (state, dir) = make_test_state_with_tempdir();
    let session_dir = dir.path().join("sessions/history-page-vm");
    std::fs::create_dir_all(&session_dir).unwrap();
    let db_path = session_dir.join("session.db");
    let mut expected = Vec::new();
    let writer = capsem_logger::DbWriter::open(&db_path, 64).unwrap();
    let execs = (0..EXECS).map(|i| (at(2 * i), format!("cmd-{i:02}"))).chain([
        (at(200), "grep 50%_off".to_string()),
        (at(201), "grep 50xyoff".to_string()),
    ]);
    for (exec_id, (timestamp, command)) in execs.enumerate() {
        expected.push((capsem_logger::format_ledger_timestamp(timestamp), command.clone()));
        writer
            .write(capsem_logger::WriteOp::ExecEvent(capsem_logger::ExecEvent {
                event_id: None,
                timestamp,
                exec_id: exec_id as u64,
                command,
                source: "api".to_string(),
                trace_id: None,
                process_name: Some("bash".to_string()),
                credential_ref: None,
            }))
            .await;
        writer
            .write(capsem_logger::WriteOp::ExecEventComplete(
                capsem_logger::ExecEventComplete {
                    exec_id: exec_id as u64,
                    exit_code: 0,
                    duration_ms: 1,
                    stdout_preview: Some(format!("out-{exec_id}")),
                    stderr_preview: None,
                    stdout_bytes: 6,
                    stderr_bytes: 0,
                    pid: None,
                },
            ))
            .await;
    }
    for i in 0..AUDITS {
        let argv = format!("argv-{i:02}");
        expected.push((capsem_logger::format_ledger_timestamp(at(2 * i + 1)), argv.clone()));
        writer
            .write(capsem_logger::WriteOp::AuditEvent(capsem_logger::AuditEvent {
                event_id: None,
                timestamp: at(2 * i + 1),
                pid: 10,
                ppid: 1,
                uid: 0,
                exe: "/usr/bin/audited".to_string(),
                comm: None,
                argv,
                cwd: None,
                tty: None,
                session_id: None,
                audit_id: None,
                exec_event_id: None,
                parent_exe: None,
                trace_id: None,
                credential_ref: None,
            }))
            .await;
    }
    writer.shutdown_blocking();
    insert_fake_instance_with_session_dir(&state, "history-page-vm", std::process::id(), session_dir);
    expected.sort_by(|left, right| right.0.cmp(&left.0));
    (build_service_router(state), dir, expected)
}

async fn history(app: &axum::Router, query: &str) -> api::HistoryResponse {
    let (status, body) = route_request(
        app.clone(),
        axum::http::Method::GET,
        &format!("/vms/history-page-vm/history?{query}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{query}: {body}");
    serde_json::from_value(body).unwrap()
}

fn commands(page: &api::HistoryResponse) -> Vec<&str> {
    page.commands.iter().map(|entry| entry.command.as_str()).collect()
}

#[tokio::test]
async fn history_pages_newest_first_across_both_layers() {
    let (app, _dir, expected) = history_app().await;
    let all = (EXECS + 2 + AUDITS) as usize;

    let page = history(&app, "limit=10&offset=10").await;
    let want: Vec<&str> = expected[10..20].iter().map(|(_, command)| command.as_str()).collect();
    assert_eq!(commands(&page), want);
    let stamps: Vec<&str> = page.commands.iter().map(|entry| entry.timestamp.as_str()).collect();
    let want_stamps: Vec<&str> = expected[10..20].iter().map(|(stamp, _)| stamp.as_str()).collect();
    assert_eq!(stamps, want_stamps);
    assert_eq!((page.total, page.has_more), (all as u64, true));

    let last = history(&app, &format!("limit=10&offset={}", all - 3)).await;
    assert_eq!(last.commands.len(), 3);
    assert!(!last.has_more);
    let past = history(&app, "offset=18446744073709551615").await;
    assert!(past.commands.is_empty() && !past.has_more);
    assert_eq!(past.total, all as u64);

    let exec = history(&app, "layer=exec&limit=5").await;
    assert_eq!(exec.total, EXECS + 2);
    assert_eq!(
        commands(&exec),
        ["grep 50xyoff", "grep 50%_off", "cmd-29", "cmd-28", "cmd-27"]
    );
    let audit = history(&app, "layer=audit&limit=2&offset=1").await;
    assert_eq!(audit.total, AUDITS);
    assert_eq!(commands(&audit), ["argv-28", "argv-27"]);
    assert!(audit
        .commands
        .iter()
        .all(|entry| entry.layer == api::HistoryLayer::Audit));
}

/// Without a search the total is the writer's counter snapshot, the same
/// numbers `/history/counts` reports, not a COUNT over the ledger.
#[tokio::test]
async fn history_total_without_search_is_the_counter_snapshot() {
    let (app, _dir, _) = history_app().await;
    let (status, counts) = route_request(
        app.clone(),
        axum::http::Method::GET,
        "/vms/history-page-vm/history/counts",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{counts}");
    let exec = counts["exec_count"].as_u64().unwrap();
    let audit = counts["audit_count"].as_u64().unwrap();
    assert_eq!(history(&app, "limit=1").await.total, exec + audit);
    assert_eq!(history(&app, "layer=exec&limit=1").await.total, exec);
    assert_eq!(history(&app, "layer=audit&limit=1").await.total, audit);
}

#[tokio::test]
async fn history_search_filters_and_counts_only_matches() {
    let (app, _dir, _) = history_app().await;

    let first = history(&app, "search=cmd-1&limit=4").await;
    assert_eq!(commands(&first), ["cmd-19", "cmd-18", "cmd-17", "cmd-16"]);
    assert_eq!((first.total, first.has_more), (10, true));
    let rest = history(&app, "search=cmd-1&limit=4&offset=8").await;
    assert_eq!(commands(&rest), ["cmd-11", "cmd-10"]);
    assert_eq!((rest.total, rest.has_more), (10, false));

    // stdout previews are searched: exec_id 7 printed "out-7".
    assert_eq!(commands(&history(&app, "search=out-7").await), ["cmd-07"]);
    // The details object is searched as the JSON the response carries.
    let exe = history(&app, "search=%22exe%22%3A%22%2Fusr%2Fbin%2Faudited%22&limit=1").await;
    assert_eq!(exe.total, AUDITS);
    assert_eq!(exe.commands[0].layer, api::HistoryLayer::Audit);
    // Layer and search compose.
    assert_eq!(history(&app, "search=argv-2&layer=exec").await.total, 0);
    assert_eq!(history(&app, "search=argv-2&layer=audit").await.total, 10);
    // Case-sensitive, as it always was.
    assert_eq!(history(&app, "search=CMD-1").await.total, 0);
}

/// `%` and `_` in a search are the characters themselves, not wildcards.
#[tokio::test]
async fn history_search_wildcards_are_literal() {
    let (app, _dir, _) = history_app().await;
    let page = history(&app, "search=50%25_off").await;
    assert_eq!(commands(&page), ["grep 50%_off"]);
    assert_eq!(page.total, 1);
    assert_eq!(history(&app, "search=%25").await.total, 1);
    assert_eq!(history(&app, "search=cmd_0").await.total, 0);
}
