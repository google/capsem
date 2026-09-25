//! `GET /vms/{id}/timeline` reads a window per layer from `since`, not the
//! oldest rows of the whole ledger.

use std::time::{Duration, SystemTime};

use api::TimelineLayer::{Exec, Fs, Model, Net, Tool};

use super::*;

/// More old rows than any fixed read window the route ever had.
const OLD_ROWS: usize = 60_000;

fn ago(secs: u64) -> String {
    capsem_logger::format_ledger_timestamp(SystemTime::now() - Duration::from_secs(secs))
}

/// A ledger created by the writer, then seeded directly: tens of thousands
/// of old exec rows, one tool call that never recorded its time, and one
/// recent row per layer, oldest first exec, tool, net, fs, model.
async fn timeline_app() -> (axum::Router, tempfile::TempDir) {
    let (state, dir) = make_test_state_with_tempdir();
    let session_dir = dir.path().join("sessions/timeline-window-vm");
    std::fs::create_dir_all(&session_dir).unwrap();
    let db_path = session_dir.join("session.db");
    let seed_path = db_path.clone();
    tokio::task::spawn_blocking(move || {
        capsem_logger::DbWriter::open(&seed_path, 1)
            .unwrap()
            .shutdown_blocking();
        let mut conn = rusqlite::Connection::open(&seed_path).unwrap();
        let tx = conn.transaction().unwrap();
        {
            let mut old = tx
                .prepare("INSERT INTO exec_events (timestamp, exec_id, command) VALUES (?1, ?2, 'old')")
                .unwrap();
            for row in 0..OLD_ROWS {
                let stamp = format!(
                    "2020-01-01T{:02}:{:02}:{:02}.000000Z",
                    row / 3600 % 24,
                    row / 60 % 60,
                    row % 60
                );
                old.execute(rusqlite::params![stamp, row as i64]).unwrap();
            }
        }
        tx.execute(
            "INSERT INTO tool_calls (timestamp, call_index, call_id, tool_name, origin) \
             VALUES ('', 0, 'undated', 'bash', 'native')",
            [],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO exec_events (timestamp, exec_id, command, trace_id) VALUES (?1, 900000, 'recent', 'trace-a')",
            [ago(50)],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO tool_calls (timestamp, call_index, call_id, tool_name, origin, trace_id) \
             VALUES (?1, 0, 'recent', 'bash', 'native', 'trace-b')",
            [ago(40)],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO net_events (timestamp, domain, decision) VALUES (?1, 'recent.example', 'allowed')",
            [ago(30)],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO fs_events (timestamp, action, path, trace_id) VALUES (?1, 'write', '/recent', 'trace-a')",
            [ago(20)],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO model_calls (timestamp, provider, model, method, path, trace_id) \
             VALUES (?1, 'anthropic', 'recent', 'POST', '/v1/messages', 'trace-b')",
            [ago(10)],
        )
        .unwrap();
        tx.commit().unwrap();
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);").unwrap();
    })
    .await
    .unwrap();
    insert_fake_instance_with_session_dir(&state, "timeline-window-vm", std::process::id(), session_dir);
    (build_service_router(state), dir)
}

async fn timeline(app: &axum::Router, query: &str) -> Vec<api::TimelineEvent> {
    let (status, body) = route_request(
        app.clone(),
        axum::http::Method::GET,
        &format!("/vms/timeline-window-vm/timeline?{query}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{query}: {body}");
    serde_json::from_value::<api::TimelineResponse>(body).unwrap().events
}

fn layers(events: &[api::TimelineEvent]) -> Vec<api::TimelineLayer> {
    events.iter().map(|event| event.layer).collect()
}

/// The route used to read the oldest rows of the whole ledger and filter
/// them in memory, so on a long session a recent `since` returned nothing.
#[tokio::test]
async fn timeline_since_reaches_recent_rows_past_a_long_history() {
    let (app, _dir) = timeline_app().await;

    let recent = timeline(&app, "since=1h").await;
    assert_eq!(layers(&recent), [Exec, Tool, Net, Fs, Model]);
    assert_eq!(recent[0].summary, "recent");
    assert!(recent.windows(2).all(|pair| pair[0].timestamp <= pair[1].timestamp));

    assert_eq!(layers(&timeline(&app, "since=1h&layers=fs,net").await), [Net, Fs]);
    assert_eq!(layers(&timeline(&app, "since=1h&layers=net,net").await), [Net]);
    assert_eq!(layers(&timeline(&app, "since=1h&limit=2").await), [Exec, Tool]);
    // A trace keeps its own rows and the ones that carry no trace at all.
    let traced = timeline(&app, "since=1h&trace_id=trace-a").await;
    assert_eq!(layers(&traced), [Exec, Net, Fs]);
    assert!(timeline(&app, "since=45s&layers=exec").await.is_empty());
}

#[tokio::test]
async fn timeline_without_since_starts_at_the_oldest_row() {
    let (app, _dir) = timeline_app().await;
    let oldest = timeline(&app, "limit=3").await;
    // A tool call without a recorded time reads as the epoch and sorts first.
    assert_eq!(oldest[0].layer, Tool);
    assert_eq!(oldest[0].timestamp, "1970-01-01T00:00:00Z");
    assert_eq!(&oldest[1].timestamp, "2020-01-01T00:00:00.000000Z");
    assert_eq!(&oldest[2].timestamp, "2020-01-01T00:00:01.000000Z");
    assert_eq!(timeline(&app, "").await.len(), 200);
    assert_eq!(timeline(&app, "limit=100000").await.len(), 2000);
    // A cutoff at the epoch still includes the undated call, as it did when
    // the filter compared the displayed time.
    let epoch = timeline(&app, "since=1970-01-01T00:00:00Z&layers=tool").await;
    assert_eq!(epoch.len(), 2);
    assert_eq!(epoch[0].timestamp, "1970-01-01T00:00:00Z");
}

/// A cutoff inside a second keeps that second's rows: ledger timestamps carry
/// microseconds, and `12:00:00Z` sorts after `12:00:00.5Z`.
#[tokio::test]
async fn timeline_since_includes_rows_in_the_cutoff_second() {
    let (app, _dir) = timeline_app().await;
    let events = timeline(&app, "since=2020-01-01T00:00:05Z&layers=exec&limit=2").await;
    assert_eq!(events[0].timestamp, "2020-01-01T00:00:05.000000Z");
}

/// Only the requested layers are read. Each window's plan is held by the
/// route statement registry in `tests/route_query_plans.rs`.
#[test]
fn timeline_reads_only_the_requested_layers() {
    assert_eq!(
        crate::ledger_routes::timeline::timeline_sql(&[Net])
            .matches("SELECT")
            .count(),
        2
    );
}
