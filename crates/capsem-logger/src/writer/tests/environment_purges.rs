use super::*;
use crate::events::environment::purge::*;
use crate::events::environment::request::{EnvironmentRequestPayload, PreparedEnvironmentRequest};
use crate::events::{EnvironmentRevisionState, EnvironmentRevisionTransition, PreparedEnvironmentRevision};

const SELECTED: &str = "purge-selected-value-canary";
mod rewrite;
fn id(value: u64) -> String {
    format!("00000000-0000-4000-8000-{value:012x}")
}
fn revision(environment: u64, number: u64, previous: Option<u64>, value: Option<&str>) -> PreparedEnvironmentRevision {
    let mut specification: capsem_config::environment::EnvironmentSpecification =
        toml::from_str("name='development'").unwrap();
    specification.variables.insert("KEEP".into(), "unrelated value".into());
    if let Some(value) = value {
        specification.variables.insert("WRONG".into(), value.into());
    }
    PreparedEnvironmentRevision {
        environment_id: id(environment),
        revision_id: id(number),
        previous_revision_id: previous.map(id),
        fork_parent: None,
        timestamp_unix_ms: number as i64,
        actor: "operator".into(),
        reason: "purge fixture".into(),
        request_id: None,
        payload_event_id: format!("{number:012x}"),
        specification: Box::new(specification),
    }
}
fn intent() -> PreparedEnvironmentVariablePurge {
    PreparedEnvironmentVariablePurge {
        purge_id: id(50),
        environment_id: id(2),
        source_revision_id: id(3),
        variable_name: "WRONG".into(),
        timestamp_unix_ms: 50,
        actor: "operator".into(),
    }
}
async fn durable(writer: &DbWriter, op: WriteOp) {
    writer.write_checked(op).await.unwrap();
    writer.flush_checked().await.unwrap();
}
async fn rejected(writer: &DbWriter, op: WriteOp) {
    writer.write_checked(op).await.unwrap();
    assert!(writer.flush_checked().await.is_err());
}
async fn committed(writer: &DbWriter, record: PreparedEnvironmentRevision) {
    durable(writer, WriteOp::PrepareEnvironmentRevision(record.clone())).await;
    durable(
        writer,
        WriteOp::EnvironmentRevisionTransition(EnvironmentRevisionTransition {
            environment_id: record.environment_id,
            revision_id: record.revision_id,
            state: EnvironmentRevisionState::Committed,
            timestamp_unix_ms: 20,
            actor: "operator".into(),
        }),
    )
    .await;
}
async fn owner(path: &Path, clean: bool) -> DbWriter {
    let writer = DbWriter::open(path, 8).unwrap();
    for record in [
        revision(2, 3, None, Some(SELECTED)),
        revision(2, 4, Some(3), Some("different legitimate value")),
        revision(2, 5, Some(4), Some(SELECTED)),
    ] {
        committed(&writer, record).await;
    }
    if clean {
        committed(&writer, revision(2, 6, Some(5), None)).await;
    }
    // The same value in another environment is outside this explicit scope.
    committed(&writer, revision(8, 10, None, Some(SELECTED))).await;
    writer
}

#[tokio::test]
async fn variable_purge_plan_is_durable_exact_and_does_not_change_values_before_publication() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let writer = owner(&path, true).await;
    committed(&writer, revision(2, 7, Some(6), Some("different legitimate value"))).await;
    let draft = toml::from_str(&format!(
        "name='development'\n[variables]\nWRONG='{SELECTED}'\nKEEP='unrelated value'\n"
    ))
    .unwrap();
    durable(
        &writer,
        WriteOp::PrepareEnvironmentRequest(PreparedEnvironmentRequest {
            request_id: id(40),
            environment_id: id(2),
            base_revision_id: Some(id(6)),
            requester: None,
            timestamp_unix_ms: 40,
            expires_at_unix_ms: 10000,
            actor: "operator".into(),
            reason: "pending draft".into(),
            payload_event_id: "000000000028".into(),
            payload: Box::new(EnvironmentRequestPayload::Change {
                specification: Box::new(draft),
            }),
        }),
    )
    .await;
    durable(&writer, WriteOp::PrepareEnvironmentVariablePurge(intent())).await;
    durable(&writer, WriteOp::PrepareEnvironmentVariablePurge(intent())).await;
    writer.shutdown_blocking();
    let writer = DbWriter::open(&path, 8).unwrap();
    durable(&writer, WriteOp::PrepareEnvironmentVariablePurge(intent())).await;
    let reader = crate::DbHandle::open_external_reader(&path).unwrap();
    let plans = reader
        .environment_variable_purges(crate::db::EnvironmentVariablePurgeQuery::Get { purge_id: id(50) })
        .await
        .unwrap();
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].intent, intent());
    assert_eq!(plans[0].state, EnvironmentVariablePurgeState::Prepared);
    assert_eq!((plans[0].revision_count, plans[0].request_count), (2, 1));
    assert!(!format!("{:?}", plans[0]).contains(SELECTED));
    for number in [3, 5] {
        let snapshot = reader
            .environment_revision_snapshot(&id(2), &id(number))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(snapshot.specification.variables["WRONG"], SELECTED);
        assert!(snapshot.record.replayable);
    }
    let conn = Connection::open(&path).unwrap();
    let mut statement = conn
        .prepare("SELECT revision_id FROM environment_revision_purges ORDER BY revision_id")
        .unwrap();
    let ids = statement
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(ids, [id(3), id(5)]);
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM host_events WHERE kind='environment_variable_purge_prepared'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    let mut changed = intent();
    changed.source_revision_id = id(4);
    rejected(&writer, WriteOp::PrepareEnvironmentVariablePurge(changed)).await;
}

#[tokio::test]
async fn variable_purge_plan_refuses_non_operator_current_unavailable_and_inflight_inputs() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let writer = owner(&path, false).await;
    rejected(&writer, WriteOp::PrepareEnvironmentVariablePurge(intent())).await;
    committed(&writer, revision(2, 6, Some(5), None)).await;
    let mut cases = Vec::new();
    let mut record = intent();
    record.actor = format!("agent:{}", id(20));
    cases.push(record);
    let mut record = intent();
    record.source_revision_id = id(10);
    cases.push(record);
    let mut record = intent();
    record.variable_name = "ABSENT".into();
    cases.push(record);
    let mut record = intent();
    record.variable_name = "../WRONG".into();
    cases.push(record);
    for record in cases {
        rejected(&writer, WriteOp::PrepareEnvironmentVariablePurge(record)).await;
    }
    durable(
        &writer,
        WriteOp::PrepareEnvironmentRevision(revision(2, 7, Some(6), None)),
    )
    .await;
    rejected(&writer, WriteOp::PrepareEnvironmentVariablePurge(intent())).await;
    durable(
        &writer,
        WriteOp::EnvironmentRevisionTransition(EnvironmentRevisionTransition {
            environment_id: id(2),
            revision_id: id(7),
            state: EnvironmentRevisionState::RecoveredUnapplied,
            timestamp_unix_ms: 30,
            actor: "system".into(),
        }),
    )
    .await;
    let conn = Connection::open(&path).unwrap();
    conn.execute("DELETE FROM event_body_blobs WHERE event_id='000000000003'", [])
        .unwrap();
    rejected(&writer, WriteOp::PrepareEnvironmentVariablePurge(intent())).await;
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM environment_variable_purges", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn variable_purge_plan_refuses_missing_scope_payload_and_staged_proposals() {
    for staged in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("host.db");
        let writer = owner(&path, true).await;
        let conn = Connection::open(&path).unwrap();
        if staged {
            let draft = toml::from_str(&format!("name='development'\n[variables]\nWRONG='{SELECTED}'\n")).unwrap();
            writer
                .write_batch(vec![
                    WriteOp::PrepareEnvironmentRequest(PreparedEnvironmentRequest {
                        request_id: id(40),
                        environment_id: id(2),
                        base_revision_id: Some(id(6)),
                        requester: None,
                        timestamp_unix_ms: 40,
                        expires_at_unix_ms: 10000,
                        actor: "operator".into(),
                        reason: "unflushed draft".into(),
                        payload_event_id: "000000000028".into(),
                        payload: Box::new(EnvironmentRequestPayload::Change {
                            specification: Box::new(draft),
                        }),
                    }),
                    WriteOp::PrepareEnvironmentVariablePurge(intent()),
                ])
                .await
                .unwrap();
            assert!(
                writer.flush_checked().await.is_err(),
                "purge target planning cannot skip an unflushed proposal"
            );
        } else {
            conn.execute("DELETE FROM event_body_blobs WHERE event_id='000000000005'", [])
                .unwrap();
            rejected(&writer, WriteOp::PrepareEnvironmentVariablePurge(intent())).await;
        }
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM environment_variable_purges", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}

#[tokio::test]
async fn variable_purge_plan_retains_prepared_sources_after_environment_archival() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let writer = owner(&path, true).await;
    durable(&writer, WriteOp::PrepareEnvironmentVariablePurge(intent())).await;
    let conn = Connection::open(&path).unwrap();
    conn.execute(
        "DELETE FROM environment_revision_references WHERE owner_kind='current_environment' AND owner_id=?1",
        [id(2)],
    )
    .unwrap();
    writer.retain_bodies_since("9999-01-01T00:00:00Z").await.unwrap();
    let reader = crate::DbHandle::open_external_reader(&path).unwrap();
    for number in [3, 5] {
        assert_eq!(
            reader
                .environment_revision_snapshot(&id(2), &id(number))
                .await
                .unwrap()
                .unwrap()
                .specification
                .variables["WRONG"],
            SELECTED
        );
    }
}

#[tokio::test]
async fn variable_purge_plan_refuses_a_canonical_pointer_to_another_environment() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let writer = owner(&path, false).await;
    committed(&writer, revision(9, 11, None, None)).await;
    let conn = Connection::open(&path).unwrap();
    conn.execute("UPDATE environment_revision_references SET environment_id=?1,revision_id=?2 WHERE owner_kind='current_environment' AND owner_id=?3",
        rusqlite::params![id(9),id(11),id(2)]).unwrap();
    rejected(&writer, WriteOp::PrepareEnvironmentVariablePurge(intent())).await;
}

#[tokio::test]
async fn variable_purge_plan_covers_a_fork_result_proposal_without_purging_parent_history() {
    use crate::events::environment::request::{
        EnvironmentRequestDecision, EnvironmentRequestDecisionKind, EnvironmentRequestResult, EnvironmentRequestTarget,
    };
    use crate::events::EnvironmentRevisionParent;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let writer = owner(&path, true).await;
    let draft = toml::from_str(&format!(
        "name='development'\n[variables]\nWRONG='{SELECTED}'\nKEEP='unrelated value'\n"
    ))
    .unwrap();
    durable(
        &writer,
        WriteOp::PrepareEnvironmentRequest(PreparedEnvironmentRequest {
            request_id: id(40),
            environment_id: id(2),
            base_revision_id: Some(id(6)),
            requester: Some(EnvironmentRequestTarget {
                session_id: id(30),
                generation: id(31),
            }),
            timestamp_unix_ms: 40,
            expires_at_unix_ms: 10000,
            actor: format!("agent:{}", id(20)),
            reason: "fork proposal".into(),
            payload_event_id: "000000000028".into(),
            payload: Box::new(EnvironmentRequestPayload::Change {
                specification: Box::new(draft),
            }),
        }),
    )
    .await;
    durable(
        &writer,
        WriteOp::EnvironmentRequestDecision(EnvironmentRequestDecision {
            request_id: id(40),
            timestamp_unix_ms: 41,
            actor: "operator".into(),
            decision: EnvironmentRequestDecisionKind::Approve,
            reason: "approved fork".into(),
        }),
    )
    .await;
    let parent = EnvironmentRevisionParent {
        environment_id: id(2),
        revision_id: id(6),
    };
    let mut fork = revision(6, 7, None, Some(SELECTED));
    fork.fork_parent = Some(parent.clone());
    fork.request_id = Some(id(40));
    committed(&writer, fork).await;
    durable(
        &writer,
        WriteOp::EnvironmentRequestResult(EnvironmentRequestResult {
            request_id: id(40),
            result: EnvironmentRevisionParent {
                environment_id: id(6),
                revision_id: id(7),
            },
            timestamp_unix_ms: 44,
            actor: "operator".into(),
        }),
    )
    .await;
    let mut clean = revision(6, 9, Some(7), None);
    clean.fork_parent = Some(parent);
    committed(&writer, clean).await;
    let mut purge = intent();
    purge.environment_id = id(6);
    purge.source_revision_id = id(7);
    durable(&writer, WriteOp::PrepareEnvironmentVariablePurge(purge)).await;
    let conn = Connection::open(&path).unwrap();
    assert_eq!(
        conn.query_row("SELECT revision_id FROM environment_revision_purges", [], |row| row
            .get::<_, String>(
            0
        ))
        .unwrap(),
        id(7)
    );
    assert_eq!(
        conn.query_row("SELECT request_id FROM environment_request_purges", [], |row| row
            .get::<_, String>(0))
            .unwrap(),
        id(40)
    );
}

#[tokio::test]
async fn variable_purge_plan_queries_bound_pages_and_fail_on_broken_metadata() {
    use crate::db::EnvironmentVariablePurgeQuery;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let writer = owner(&path, true).await;
    durable(&writer, WriteOp::PrepareEnvironmentVariablePurge(intent())).await;
    let reader = crate::DbHandle::open_external_reader(&path).unwrap();
    let page = |environment, after_id, limit| EnvironmentVariablePurgeQuery::Page {
        environment_id: Some(id(environment)),
        after_id,
        limit,
    };
    let first = reader.environment_variable_purges(page(2, 0, 1)).await.unwrap();
    assert_eq!(first.len(), 1);
    assert!(reader
        .environment_variable_purges(page(2, first[0].cursor, 1))
        .await
        .unwrap()
        .is_empty());
    assert!(reader
        .environment_variable_purges(page(8, 0, 1))
        .await
        .unwrap()
        .is_empty());
    for query in [
        page(2, 0, 0),
        page(2, -1, 1),
        EnvironmentVariablePurgeQuery::Get { purge_id: "bad".into() },
    ] {
        let count = reader.reader_requests();
        assert!(reader.environment_variable_purges(query).await.is_err());
        assert_eq!(reader.reader_requests(), count);
    }
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch("UPDATE environment_variable_purges SET variable_name='../WRONG'")
        .unwrap();
    assert!(reader.environment_variable_purges(page(2, 0, 1)).await.is_err());
    conn.execute_batch("UPDATE environment_variable_purges SET variable_name='WRONG',revision_count=9223372036854775807,request_count=1").unwrap();
    assert!(
        reader.environment_variable_purges(page(2, 0, 1)).await.is_err(),
        "corrupt stored counts must not overflow rewrite accounting"
    );
    conn.execute_batch("UPDATE environment_variable_purges SET variable_name='WRONG'; ALTER TABLE environment_variable_purges RENAME TO missing_variable_purges").unwrap();
    assert!(reader
        .environment_variable_purges(page(2, 0, 1))
        .await
        .unwrap_err()
        .contains("no such table"));
}

#[test]
fn variable_purge_status_queries_do_not_walk_target_history() {
    let conn = Connection::open_in_memory().unwrap();
    crate::schema::create_tables(&conn).unwrap();
    for predicate in ["p.purge_id=?1", "p.environment_id=?1 AND p.id>0 ORDER BY p.id LIMIT 10"] {
        let mut statement = conn
            .prepare(&format!(
                "EXPLAIN QUERY PLAN SELECT {} FROM environment_variable_purges p WHERE {predicate}",
                crate::db::PURGE_COLUMNS
            ))
            .unwrap();
        let plan = statement
            .query_map([id(2)], |row| row.get::<_, String>(3))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .join("\n");
        assert!(
            !plan.contains("CORRELATED"),
            "purge polling must not recount history: {plan}"
        );
        assert!(
            !plan.contains("environment_revision_purges") && !plan.contains("environment_request_purges"),
            "{plan}"
        );
        assert!(plan.contains("SEARCH p USING"), "{plan}");
    }
}
