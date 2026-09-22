//! The writer's memory schema holds only what it has not flushed yet.
//!
//! It used to copy the whole ledger into RAM at open and never let a flushed
//! row go, so capsem-process grew with the session for as long as it ran
//! (#213). These tests hold the replacement to its three promises: memory is
//! emptied by every flush, nothing is copied at open, and a row that has left
//! memory is still the row later writes find -- by id, by exec id, and by the
//! ledgers' unique keys.

use std::path::Path;

use super::*;
use crate::schema::memory_row_count_for_tests as mem_rows;

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap()
}

fn disk(path: &Path) -> rusqlite::Connection {
    rusqlite::Connection::open(path).unwrap()
}

fn disk_count(path: &Path, sql: &str) -> i64 {
    disk(path).query_row(sql, [], |row| row.get(0)).unwrap()
}

fn fs_op(idx: usize) -> WriteOp {
    WriteOp::FileEvent(super::file_event(
        format!("/root/unflushed/{idx}"),
        crate::events::FileAction::Read,
        Some(1),
    ))
}

fn exec_start(exec_id: u64, command: &str) -> WriteOp {
    WriteOp::ExecEvent(crate::events::ExecEvent {
        event_id: None,
        timestamp: std::time::SystemTime::now(),
        exec_id,
        command: command.into(),
        source: "api".into(),
        trace_id: Some("trace-package".into()),
        process_name: None,
        credential_ref: None,
    })
}

fn exec_complete(exec_id: u64, stdout: &str) -> WriteOp {
    WriteOp::ExecEventComplete(crate::events::ExecEventComplete {
        exec_id,
        exit_code: 0,
        duration_ms: 7_650,
        stdout_preview: Some(stdout.into()),
        stderr_preview: None,
        stdout_bytes: stdout.len() as u64,
        stderr_bytes: 0,
        pid: Some(4321),
    })
}

/// A burst far larger than one batch, drained by a barrier: every flush leaves
/// memory empty, however many batches fed it.
#[test]
fn every_flush_empties_memory_even_after_a_burst_of_many_batches() {
    const BATCH: usize = 256;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let rt = runtime();
    let writer = DbWriter::open(&path, BATCH).unwrap();

    let mut written = 0_usize;
    for (round, burst) in [5, 20 * BATCH, 3].into_iter().enumerate() {
        rt.block_on(async {
            for idx in 0..burst {
                writer.write(fs_op(written + idx)).await;
            }
            writer.flush().await;
        });
        written += burst;
        assert_eq!(
            mem_rows(&path, "fs_events"),
            0,
            "round {round}: a flushed row stayed in memory"
        );
        assert_eq!(disk_count(&path, "SELECT COUNT(*) FROM fs_events"), written as i64);
    }
}

/// No barrier at all: the writer's own interval flush is what empties memory,
/// so what it holds is bounded by one interval of traffic and not by the
/// session.
#[test]
fn the_interval_flush_empties_memory_without_a_barrier() {
    const BATCH: usize = 256;
    const BURST: usize = 12 * BATCH;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let rt = runtime();
    let writer = DbWriter::open(&path, BATCH).unwrap();
    rt.block_on(async {
        for idx in 0..BURST {
            writer.write(fs_op(idx)).await;
        }
    });

    let deadline = std::time::Instant::now() + 3 * DISK_FLUSH_INTERVAL;
    loop {
        let held = mem_rows(&path, "fs_events");
        let flushed = disk_count(&path, "SELECT COUNT(*) FROM fs_events");
        if held == 0 && flushed == BURST as i64 {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the interval flush never emptied memory: {held} rows held, {flushed} on disk"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    writer.shutdown_blocking();
}

#[test]
fn reopening_a_large_ledger_copies_nothing_into_memory() {
    const ROWS: i64 = 100_000;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    DbWriter::open(&path, 64).unwrap().shutdown_blocking();
    {
        let mut conn = disk(&path);
        crate::schema::apply_pragmas(&conn).unwrap();
        crate::schema::create_tables(&conn).unwrap();
        let tx = conn.transaction().unwrap();
        {
            let mut insert = tx
                .prepare(
                    "INSERT INTO fs_events (timestamp, action, path, kind)
                     VALUES ('2026-09-18T00:00:00Z', 'read', ?1, 'file')",
                )
                .unwrap();
            for idx in 0..ROWS {
                insert.execute([format!("/seed/{idx}")]).unwrap();
            }
        }
        tx.commit().unwrap();
    }

    let writer = DbWriter::open(&path, 64).unwrap();
    assert_eq!(
        mem_rows(&path, "fs_events"),
        0,
        "opening a ledger must not copy its rows into RAM"
    );
    drop(writer);
    assert_eq!(disk_count(&path, "SELECT COUNT(*) FROM fs_events"), ROWS);
}

#[test]
fn ids_continue_after_reopen_and_never_reuse_a_sequence_number() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let rt = runtime();
    {
        let writer = DbWriter::open(&path, 64).unwrap();
        rt.block_on(async {
            writer.write(super::minimal_model_call("trace-first")).await;
            for idx in 0..3 {
                writer.write(fs_op(idx)).await;
            }
            writer.flush().await;
        });
    }
    // A ledger whose newest rows were deleted: AUTOINCREMENT promises the
    // next id is above every id ever handed out, not above what is left.
    disk(&path)
        .execute_batch("UPDATE sqlite_sequence SET seq = 500 WHERE name = 'fs_events';")
        .unwrap();

    {
        let writer = DbWriter::open(&path, 64).unwrap();
        rt.block_on(async {
            writer.write(super::minimal_model_call("trace-second")).await;
            writer.write(fs_op(3)).await;
            writer.flush().await;
        });
    }

    let conn = disk(&path);
    let fs_ids: Vec<i64> = conn
        .prepare("SELECT id FROM fs_events ORDER BY id")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(fs_ids, vec![1, 2, 3, 501]);

    let calls: Vec<(i64, String)> = conn
        .prepare("SELECT id, trace_id FROM model_calls ORDER BY id")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        calls,
        vec![(1, "trace-first".to_string()), (2, "trace-second".to_string())],
        "the first session's call must keep its id and the second must follow it"
    );
    let orphaned: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM model_items AS item
             WHERE NOT EXISTS (
                SELECT 1 FROM model_calls AS call
                WHERE call.id = item.model_call_id AND call.trace_id IS item.trace_id
             )",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(orphaned, 0, "every model item must name the call it was written with");
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM model_items", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        2
    );
}

#[test]
fn exec_event_completion_updates_disk_after_start_was_flushed() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("exec-flushed-start.db");
    let rt = runtime();

    let writer = DbWriter::open(&db_path, 64).unwrap();
    rt.block_on(async {
        writer.write(exec_start(7, "bash /root/package-probe.sh")).await;
        writer.flush().await;
    });
    assert_eq!(
        mem_rows(&db_path, "exec_events"),
        0,
        "the start row must have left memory, so the completion has to find it on disk"
    );
    assert!(disk(&db_path)
        .query_row("SELECT exit_code FROM exec_events WHERE exec_id = 7", [], |row| {
            row.get::<_, Option<i64>>(0)
        })
        .unwrap()
        .is_none());

    rt.block_on(async {
        writer.write(exec_complete(7, "APT_OK\nNPM_OK\nUV_OK\n")).await;
        writer.flush().await;
    });
    assert_eq!(mem_rows(&db_path, "exec_events"), 0);

    let conn = disk(&db_path);
    let (event_id, exit_code, duration_ms, stdout_preview, stdout_bytes, pid): (
        String,
        i64,
        i64,
        Option<String>,
        i64,
        Option<i64>,
    ) = conn
        .query_row(
            "SELECT event_id, exit_code, duration_ms, stdout_preview, stdout_bytes, pid
             FROM exec_events WHERE exec_id = 7",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(exit_code, 0);
    assert_eq!(duration_ms, 7_650);
    assert_eq!(stdout_preview.as_deref(), Some("APT_OK\nNPM_OK\nUV_OK\n"));
    assert_eq!(stdout_bytes, 20);
    assert_eq!(pid, Some(4321));
    let archived: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM event_body_blobs
             WHERE event_id = ?1 AND source_table = 'exec_events' AND direction = 'stdout'",
            [&event_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        archived, 1,
        "the output must be archived under the start row's event id"
    );
}

#[test]
fn a_completion_never_reaches_a_previous_sessions_exec_with_the_same_id() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let rt = runtime();
    {
        let writer = DbWriter::open(&path, 64).unwrap();
        rt.block_on(async {
            writer.write(exec_start(7, "echo first-boot")).await;
            writer.write(exec_start(9, "sleep first-boot")).await;
            writer.flush().await;
        });
    }

    // Exec ids restart with the process; a resumed session reuses them. Its
    // 9 never started here, so its completion has no row to land on -- the
    // first boot's 9 is not it.
    let writer = DbWriter::open(&path, 64).unwrap();
    rt.block_on(async {
        writer.write(exec_start(7, "echo second-boot")).await;
        writer.flush().await;
        writer.write(exec_complete(7, "second\n")).await;
        writer.write(exec_complete(9, "not mine\n")).await;
        writer.flush().await;
    });
    drop(writer);

    let rows: Vec<(String, Option<i64>)> = disk(&path)
        .prepare("SELECT command, exit_code FROM exec_events WHERE exec_id IN (7, 9) ORDER BY id")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        rows,
        vec![
            ("echo first-boot".to_string(), None),
            ("sleep first-boot".to_string(), None),
            ("echo second-boot".to_string(), Some(0)),
        ],
        "the completion belongs to this session's exec, not to the older row that shares its id"
    );
}

#[test]
fn a_model_item_already_on_disk_keeps_its_first_row() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let rt = runtime();
    let writer = DbWriter::open(&path, 64).unwrap();
    rt.block_on(async {
        writer.write(super::minimal_model_call("trace-dedup")).await;
        writer.flush().await;
    });
    let first: (i64, i64) = disk(&path)
        .query_row(
            "SELECT id, model_call_id FROM model_items WHERE trace_id = 'trace-dedup'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(mem_rows(&path, "model_items"), 0);

    rt.block_on(async {
        writer.write(super::minimal_model_call("trace-dedup")).await;
        writer.flush().await;
    });
    drop(writer);

    let rows: Vec<(i64, i64)> = disk(&path)
        .prepare("SELECT id, model_call_id FROM model_items WHERE trace_id = 'trace-dedup'")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        rows,
        vec![first],
        "the repeated item must be dropped, not replace the row the first call wrote"
    );
}

#[test]
fn a_transport_event_id_already_on_disk_is_refused_not_replaced() {
    use crate::events::{TransportEvent, TransportEventKind};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let rt = runtime();
    let event = |facts: &str| {
        TransportEvent::new(
            "a1a1a1a1a1a1".into(),
            1,
            TransportEventKind::Connect,
            None,
            Some(uuid::Uuid::new_v4()),
            &serde_json::json!({ "facts": facts }),
        )
        .unwrap()
    };
    let writer = DbWriter::open(&path, 64).unwrap();
    rt.block_on(async {
        writer.write(WriteOp::TransportEvent(event("original"))).await;
        writer.flush().await;
        writer.write(WriteOp::TransportEvent(event("replayed"))).await;
        writer.flush().await;
    });
    drop(writer);

    let rows: Vec<String> = disk(&path)
        .prepare("SELECT event_json FROM transport_events WHERE event_id = 'a1a1a1a1a1a1'")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        rows,
        vec![r#"{"facts":"original"}"#.to_string()],
        "a replayed event id must be rejected as it always was, not overwrite the recorded one"
    );
}
