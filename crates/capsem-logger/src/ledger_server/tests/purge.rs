use super::*;
use crate::events::environment::purge::*;
use crate::events::{EnvironmentRevisionState, EnvironmentRevisionTransition, PreparedEnvironmentRevision};
use crate::ledger_client::LedgerClient;

fn id(n: u64) -> String {
    format!("00000000-0000-4000-8000-{n:012x}")
}

async fn prepared(server: &LedgerServer) {
    for (number, previous, value) in [(3, None, Some("purge worker canary")), (4, Some(3), None)] {
        let mut specification: capsem_config::environment::EnvironmentSpecification =
            toml::from_str("name='development'").unwrap();
        if let Some(value) = value {
            specification.variables.insert("WRONG".into(), value.into());
        }
        server
            .db
            .write(WriteOp::PrepareEnvironmentRevision(PreparedEnvironmentRevision {
                environment_id: id(2),
                revision_id: id(number),
                previous_revision_id: previous.map(id),
                fork_parent: None,
                timestamp_unix_ms: number as i64,
                actor: "operator".into(),
                reason: "fixture".into(),
                request_id: None,
                payload_event_id: format!("{number:012x}"),
                specification: Box::new(specification),
            }))
            .await
            .unwrap();
        server.db.flush().await.unwrap();
        server
            .db
            .write(WriteOp::EnvironmentRevisionTransition(EnvironmentRevisionTransition {
                environment_id: id(2),
                revision_id: id(number),
                state: EnvironmentRevisionState::Committed,
                timestamp_unix_ms: 10,
                actor: "operator".into(),
            }))
            .await
            .unwrap();
        server.db.flush().await.unwrap();
    }
    server
        .db
        .write(WriteOp::PrepareEnvironmentVariablePurge(
            PreparedEnvironmentVariablePurge {
                purge_id: id(5),
                environment_id: id(2),
                source_revision_id: id(3),
                variable_name: "WRONG".into(),
                timestamp_unix_ms: 20,
                actor: "operator".into(),
            },
        ))
        .await
        .unwrap();
    server.db.flush().await.unwrap();
}

#[tokio::test]
async fn maintainer_variable_purge_uses_the_existing_ledger_client_and_writer_barrier() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let server = Arc::new(LedgerServer::open(&path).unwrap());
    prepared(&server).await;
    let authority = grant(LedgerClientRole::Maintainer);
    let (client, worker) = UnixStream::pair().unwrap();
    let owner = Arc::clone(&server);
    let task = tokio::spawn(async move { owner.serve_client(worker, authority).await });
    let client = LedgerClient::connect(client, authority, path).await.unwrap();
    assert!(client
        .purge_variable_history(EnvironmentVariablePurgeExecution {
            purge_id: "bad".into(),
            timestamp_unix_ms: 21
        })
        .await
        .is_err());
    let execution = EnvironmentVariablePurgeExecution {
        purge_id: id(5),
        timestamp_unix_ms: 21,
    };
    let result = client.purge_variable_history(execution.clone()).await.unwrap();
    assert_eq!(result.state, EnvironmentVariablePurgeState::Committed);
    assert_eq!(client.purge_variable_history(execution).await.unwrap(), result);
    assert!(server
        .db
        .environment_revision_snapshot(&id(2), &id(3))
        .await
        .unwrap_err()
        .contains("non-replayable"));
    client.counters().await.unwrap();
    drop(client);
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn maintainer_variable_purge_uses_the_existing_remote_writer() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let server = Arc::new(LedgerServer::open(&path).unwrap());
    prepared(&server).await;
    let authority = grant(LedgerClientRole::Maintainer);
    let (client, worker) = UnixStream::pair().unwrap();
    let owner = Arc::clone(&server);
    let task = tokio::spawn(async move { owner.serve_client(worker, authority).await });
    let writer = tokio::task::spawn_blocking(move || crate::DbWriter::from_ledger_channel(client, authority, &path, 8))
        .await
        .unwrap()
        .unwrap();
    let execution = EnvironmentVariablePurgeExecution {
        purge_id: id(5),
        timestamp_unix_ms: 21,
    };
    let result = writer.purge_variable_history(execution.clone()).await.unwrap();
    assert_eq!(result.state, EnvironmentVariablePurgeState::Committed);
    assert_eq!(writer.purge_variable_history(execution).await.unwrap(), result);
    tokio::task::spawn_blocking(move || writer.shutdown_blocking())
        .await
        .unwrap();
    assert_eq!(task.await.unwrap().unwrap(), LedgerClientExit::Disconnected);
}
