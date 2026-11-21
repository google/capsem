use super::*;
use crate::db::{
    EnvironmentApplicationQuery, EnvironmentHistoryPayload, EnvironmentReferenceQuery, EnvironmentRequestQuery,
    EnvironmentRevisionQuery, EnvironmentVariablePurgeQuery,
};
use crate::ledger_client::LedgerClient;
use crate::ledger_protocol::{LedgerEnvironmentQuery, LedgerEnvironmentReply};

fn id(n: u64) -> String {
    format!("00000000-0000-4000-8000-{n:012x}")
}

#[tokio::test]
async fn environment_reads_use_the_owning_ledger_client_and_typed_db_queries() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let server = Arc::new(LedgerServer::open(&path).unwrap());
    super::purge::prepared(&server).await;
    let authority = grant(LedgerClientRole::Reader);
    let (client, worker) = UnixStream::pair().unwrap();
    let owner = Arc::clone(&server);
    let task = tokio::spawn(async move { owner.serve_client(worker, authority).await });
    let client = LedgerClient::connect(client, authority, path).await.unwrap();
    assert!(client
        .environment_read(LedgerEnvironmentQuery::Revisions {
            query: EnvironmentRevisionQuery::Page {
                environment_id: Some(id(2)),
                state: None,
                after_id: 0,
                limit: 0
            }
        })
        .await
        .is_err());
    let reply = client
        .environment_read(LedgerEnvironmentQuery::Revisions {
            query: EnvironmentRevisionQuery::Page {
                environment_id: Some(id(2)),
                state: None,
                after_id: 0,
                limit: 2,
            },
        })
        .await
        .unwrap();
    let LedgerEnvironmentReply::Revisions { records } = reply else {
        panic!("revision metadata")
    };
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].revision_id, id(3));
    let reply = client
        .environment_read(LedgerEnvironmentQuery::RevisionSnapshot {
            environment_id: id(2),
            revision_id: id(3),
        })
        .await
        .unwrap();
    let LedgerEnvironmentReply::RevisionSnapshot {
        snapshot: Some(snapshot),
    } = reply
    else {
        panic!("revision snapshot")
    };
    assert_eq!(snapshot.specification.variables["WRONG"], "purge worker canary");
    let reply = client
        .environment_read(LedgerEnvironmentQuery::RevisionHistory {
            environment_id: id(2),
            revision_id: id(3),
        })
        .await
        .unwrap();
    assert!(
        matches!(reply,LedgerEnvironmentReply::RevisionHistory {history:Some(history)} if matches!(history.payload,EnvironmentHistoryPayload::Complete {..}))
    );
    for query in [
        LedgerEnvironmentQuery::RequestSnapshot { request_id: id(99) },
        LedgerEnvironmentQuery::RequestHistory { request_id: id(99) },
    ] {
        assert!(matches!(
            client.environment_read(query).await.unwrap(),
            LedgerEnvironmentReply::RequestSnapshot { snapshot: None }
                | LedgerEnvironmentReply::RequestHistory { history: None }
        ));
    }
    let reply = client
        .environment_read(LedgerEnvironmentQuery::Requests {
            query: EnvironmentRequestQuery::Get { request_id: id(99) },
        })
        .await
        .unwrap();
    assert!(matches!(reply,LedgerEnvironmentReply::Requests {records} if records.is_empty()));
    let exact_head = client
        .environment_read(LedgerEnvironmentQuery::References {
            query: EnvironmentReferenceQuery::CurrentEnvironment { environment_id: id(2) },
        })
        .await
        .unwrap();
    assert!(matches!(&exact_head,LedgerEnvironmentReply::References {records} if records.len()==1));
    let reply = client
        .environment_read(LedgerEnvironmentQuery::References {
            query: EnvironmentReferenceQuery::CurrentEnvironments { after: None, limit: 2 },
        })
        .await
        .unwrap();
    assert_eq!(reply, exact_head);
    let reply = client
        .environment_read(LedgerEnvironmentQuery::Applications {
            query: EnvironmentApplicationQuery::Page {
                request_id: None,
                target: None,
                after_id: 0,
                limit: 1,
            },
        })
        .await
        .unwrap();
    assert!(matches!(reply,LedgerEnvironmentReply::Applications {records} if records.is_empty()));
    let reply = client
        .environment_read(LedgerEnvironmentQuery::Purges {
            query: EnvironmentVariablePurgeQuery::Get { purge_id: id(5) },
        })
        .await
        .unwrap();
    assert!(matches!(reply,LedgerEnvironmentReply::Purges {records} if records.len()==1));
    drop(client);
    task.await.unwrap().unwrap();
}

async fn durable(server: &LedgerServer, op: WriteOp) {
    server.db.write(op).await.unwrap();
    server.db.flush().await.unwrap();
}

#[tokio::test]
async fn environment_reads_preserve_nonempty_requests_applications_references_and_purged_history() {
    use crate::events::environment::application::*;
    use crate::events::environment::purge::*;
    use crate::events::environment::request::*;
    use crate::events::environment::{EnvironmentSandboxReferenceKind, EnvironmentSandboxReferenceMutation};
    use crate::events::EnvironmentRevisionParent;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let server = Arc::new(LedgerServer::open(&path).unwrap());
    super::purge::revisions(&server).await;
    let revision = EnvironmentRevisionParent {
        environment_id: id(2),
        revision_id: id(4),
    };
    let target = EnvironmentRequestTarget {
        session_id: id(30),
        generation: id(31),
    };
    for kind in [
        EnvironmentSandboxReferenceKind::Boot,
        EnvironmentSandboxReferenceKind::LivePolicy,
        EnvironmentSandboxReferenceKind::Desired,
    ] {
        durable(
            &server,
            WriteOp::EnvironmentSandboxReference(EnvironmentSandboxReferenceMutation {
                session_id: target.session_id.clone(),
                generation: target.generation.clone(),
                kind,
                expected: None,
                revision: Some(revision.clone()),
                timestamp_unix_ms: 30,
                actor: "operator".into(),
            }),
        )
        .await;
    }
    for number in [40, 41, 42] {
        let payload = if number == 41 {
            EnvironmentRequestPayload::Apply {
                revision: revision.clone(),
                targets: vec![target.clone()],
            }
        } else {
            let text = if number == 40 {
                "name='development'\n[variables]\nWRONG='purge worker canary'\nKEEP='request field'"
            } else {
                "name='development'"
            };
            EnvironmentRequestPayload::Change {
                specification: Box::new(toml::from_str(text).unwrap()),
            }
        };
        durable(
            &server,
            WriteOp::PrepareEnvironmentRequest(PreparedEnvironmentRequest {
                request_id: id(number),
                environment_id: id(2),
                base_revision_id: Some(id(4)),
                requester: None,
                timestamp_unix_ms: 40,
                expires_at_unix_ms: 10000,
                actor: "operator".into(),
                reason: "typed reads".into(),
                payload_event_id: format!("{number:012x}"),
                payload: Box::new(payload),
            }),
        )
        .await;
        if number != 42 {
            durable(
                &server,
                WriteOp::EnvironmentRequestDecision(EnvironmentRequestDecision {
                    request_id: id(number),
                    timestamp_unix_ms: 41,
                    actor: "operator".into(),
                    decision: EnvironmentRequestDecisionKind::Approve,
                    reason: "original review".into(),
                }),
            )
            .await;
        }
    }
    durable(
        &server,
        WriteOp::EnvironmentRequestResult(EnvironmentRequestResult {
            request_id: id(41),
            result: revision.clone(),
            timestamp_unix_ms: 42,
            actor: "operator".into(),
        }),
    )
    .await;
    durable(
        &server,
        WriteOp::EnvironmentApplication(EnvironmentApplicationMutation {
            request_id: id(41),
            target: target.clone(),
            expected_version: None,
            expected_references: EnvironmentSandboxReferences {
                boot: Some(revision.clone()),
                live_policy: Some(revision.clone()),
                desired: Some(revision.clone()),
            },
            timestamp_unix_ms: 43,
            actor: "operator".into(),
            state: EnvironmentApplicationState::Pending,
            composition: EnvironmentApplicationComposition {
                boot: revision.clone(),
                live_policy: revision.clone(),
                desired: revision.clone(),
                corporate_policy_id: "corp-current".into(),
                pending_restart: false,
            },
            failure_reason: None,
        }),
    )
    .await;
    durable(
        &server,
        WriteOp::PrepareEnvironmentVariablePurge(PreparedEnvironmentVariablePurge {
            purge_id: id(5),
            environment_id: id(2),
            source_revision_id: id(3),
            variable_name: "WRONG".into(),
            timestamp_unix_ms: 50,
            actor: "operator".into(),
        }),
    )
    .await;
    let authority = grant(LedgerClientRole::Maintainer);
    let (connection, worker) = UnixStream::pair().unwrap();
    let owner = Arc::clone(&server);
    let task = tokio::spawn(async move { owner.serve_client(worker, authority).await });
    let client = LedgerClient::connect(connection, authority, path).await.unwrap();
    let reply = client
        .environment_read(LedgerEnvironmentQuery::Requests {
            query: EnvironmentRequestQuery::Page {
                environment_id: Some(id(2)),
                requester: None,
                pending_only: false,
                after_id: 0,
                limit: 3,
            },
        })
        .await
        .unwrap();
    assert!(matches!(reply,LedgerEnvironmentReply::Requests {records} if records.len()==3));
    let reply = client
        .environment_read(LedgerEnvironmentQuery::RequestSnapshot { request_id: id(40) })
        .await
        .unwrap();
    assert!(
        matches!(reply,LedgerEnvironmentReply::RequestSnapshot {snapshot:Some(snapshot)} if snapshot.record.state==EnvironmentRequestState::Approved)
    );
    let reply = client
        .environment_read(LedgerEnvironmentQuery::RequestHistory { request_id: id(40) })
        .await
        .unwrap();
    assert!(
        matches!(reply,LedgerEnvironmentReply::RequestHistory {history:Some(history)} if matches!(history.payload,EnvironmentHistoryPayload::Complete {..}))
    );
    let reply = client
        .environment_read(LedgerEnvironmentQuery::Applications {
            query: EnvironmentApplicationQuery::Get {
                request_id: id(41),
                target: target.clone(),
            },
        })
        .await
        .unwrap();
    let LedgerEnvironmentReply::Applications { records } = reply else {
        panic!("application")
    };
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].version, 1);
    assert_eq!(records[0].composition.desired, revision);
    let reply = client
        .environment_read(LedgerEnvironmentQuery::References {
            query: EnvironmentReferenceQuery::Sandbox {
                session_id: target.session_id,
                generation: target.generation,
            },
        })
        .await
        .unwrap();
    assert!(matches!(reply,LedgerEnvironmentReply::References {records} if records.len()==3));
    client
        .purge_variable_history(EnvironmentVariablePurgeExecution {
            purge_id: id(5),
            timestamp_unix_ms: 51,
        })
        .await
        .unwrap();
    let reply = client
        .environment_read(LedgerEnvironmentQuery::RevisionHistory {
            environment_id: id(2),
            revision_id: id(3),
        })
        .await
        .unwrap();
    let LedgerEnvironmentReply::RevisionHistory { history: Some(history) } = reply else {
        panic!("partial revision")
    };
    assert!(
        matches!(history.payload,EnvironmentHistoryPayload::Purged {value,purged_variables} if !value.variables.contains_key("WRONG")&&purged_variables.len()==1)
    );
    let reply = client
        .environment_read(LedgerEnvironmentQuery::RequestHistory { request_id: id(40) })
        .await
        .unwrap();
    let LedgerEnvironmentReply::RequestHistory { history: Some(history) } = reply else {
        panic!("partial request")
    };
    assert_eq!(history.record.state, EnvironmentRequestState::Stale);
    assert_eq!(history.record.decision.as_ref().unwrap().reason, "original review");
    assert!(
        matches!(history.payload,EnvironmentHistoryPayload::Purged {purged_variables,..} if purged_variables.len()==1)
    );
    let reply = client
        .environment_read(LedgerEnvironmentQuery::Requests {
            query: EnvironmentRequestQuery::Page {
                environment_id: Some(id(2)),
                requester: None,
                pending_only: true,
                after_id: 0,
                limit: 3,
            },
        })
        .await
        .unwrap();
    assert!(
        matches!(reply,LedgerEnvironmentReply::Requests {records} if records.len()==1&&records[0].request_id==id(42))
    );
    assert!(client
        .environment_read(LedgerEnvironmentQuery::RevisionSnapshot {
            environment_id: id(2),
            revision_id: id(3)
        })
        .await
        .unwrap_err()
        .contains("non-replayable"));
    drop(client);
    task.await.unwrap().unwrap();
}
