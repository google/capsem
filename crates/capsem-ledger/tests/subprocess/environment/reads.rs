use super::*;
use capsem_logger::db::{
    EnvironmentApplicationQuery, EnvironmentHistoryPayload, EnvironmentReferenceQuery, EnvironmentRequestQuery,
    EnvironmentRevisionQuery, EnvironmentVariablePurgeQuery,
};
use capsem_logger::ledger_protocol::{LedgerEnvironmentQuery as Q, LedgerEnvironmentReply as R};

pub(super) async fn complete(client: &LedgerClient) {
    let reply = client
        .environment_read(Q::Revisions {
            query: EnvironmentRevisionQuery::Page {
                environment_id: Some(id(2)),
                state: None,
                after_id: 0,
                limit: 10,
            },
        })
        .await
        .unwrap();
    assert!(matches!(reply,R::Revisions {records} if records.len()==4&&records[0].revision_id==id(3)));
    let reply = client
        .environment_read(Q::RevisionSnapshot {
            environment_id: id(2),
            revision_id: id(3),
        })
        .await
        .unwrap();
    let R::RevisionSnapshot {
        snapshot: Some(snapshot),
    } = reply
    else {
        panic!("native full revision")
    };
    assert_eq!(snapshot.specification.variables["WRONG"], SELECTED);
    let reply = client
        .environment_read(Q::RevisionHistory {
            environment_id: id(2),
            revision_id: id(3),
        })
        .await
        .unwrap();
    assert!(
        matches!(reply,R::RevisionHistory {history:Some(history)} if matches!(history.payload,EnvironmentHistoryPayload::Complete {..}))
    );
    let reply = client
        .environment_read(Q::Requests {
            query: EnvironmentRequestQuery::Get { request_id: id(40) },
        })
        .await
        .unwrap();
    assert!(
        matches!(reply,R::Requests {records} if records.len()==1&&records[0].state==EnvironmentRequestState::Approved)
    );
    let reply = client
        .environment_read(Q::RequestSnapshot { request_id: id(40) })
        .await
        .unwrap();
    let R::RequestSnapshot {
        snapshot: Some(snapshot),
    } = reply
    else {
        panic!("native full request")
    };
    let EnvironmentRequestPayload::Change { specification } = snapshot.payload.as_ref() else {
        panic!("native change")
    };
    assert_eq!(specification.variables["WRONG"], SELECTED);
    let reply = client
        .environment_read(Q::RequestHistory { request_id: id(40) })
        .await
        .unwrap();
    assert!(
        matches!(reply,R::RequestHistory {history:Some(history)} if matches!(history.payload,EnvironmentHistoryPayload::Complete {..}))
    );
    let reply = client
        .environment_read(Q::References {
            query: EnvironmentReferenceQuery::CurrentEnvironment { environment_id: id(2) },
        })
        .await
        .unwrap();
    assert!(matches!(reply,R::References {records} if records.len()==1));
    let reply = client
        .environment_read(Q::Applications {
            query: EnvironmentApplicationQuery::Page {
                request_id: None,
                target: None,
                after_id: 0,
                limit: 1,
            },
        })
        .await
        .unwrap();
    assert!(matches!(reply,R::Applications {records} if records.is_empty()));
    let reply = client
        .environment_read(Q::Purges {
            query: EnvironmentVariablePurgeQuery::Get { purge_id: id(50) },
        })
        .await
        .unwrap();
    assert!(
        matches!(reply,R::Purges {records} if records.len()==1&&records[0].state==EnvironmentVariablePurgeState::Prepared)
    );
}

pub(super) async fn partial(client: &LedgerClient, state: EnvironmentVariablePurgeState) {
    let reply = client
        .environment_read(Q::RevisionHistory {
            environment_id: id(2),
            revision_id: id(3),
        })
        .await
        .unwrap();
    let R::RevisionHistory { history: Some(history) } = reply else {
        panic!("native partial revision")
    };
    assert!(!history.record.replayable);
    let EnvironmentHistoryPayload::Purged {
        value,
        purged_variables,
    } = &history.payload
    else {
        panic!("native partial values")
    };
    assert!(!value.variables.contains_key("WRONG"));
    assert_eq!(value.variables["KEEP"], "unrelated retained field");
    assert_eq!(purged_variables.len(), 1);
    assert_eq!(purged_variables[0].state, state);
    let reply = client
        .environment_read(Q::RequestHistory { request_id: id(40) })
        .await
        .unwrap();
    let R::RequestHistory { history: Some(history) } = reply else {
        panic!("native partial request")
    };
    assert_eq!(history.record.state, EnvironmentRequestState::Stale);
    assert!(!history.record.replayable);
    assert_eq!(
        history.record.decision.as_ref().unwrap().reason,
        "original native approval"
    );
    assert!(history.record.invalidation.is_some());
    let EnvironmentHistoryPayload::Purged {
        value,
        purged_variables,
    } = &history.payload
    else {
        panic!("native partial intent")
    };
    let EnvironmentRequestPayload::Change { specification } = value.as_ref() else {
        panic!("native change")
    };
    assert!(!specification.variables.contains_key("WRONG"));
    assert_eq!(specification.variables["KEEP"], "proposal field");
    assert_eq!(purged_variables[0].state, state);
    let reply = client
        .environment_read(Q::Purges {
            query: EnvironmentVariablePurgeQuery::Get { purge_id: id(50) },
        })
        .await
        .unwrap();
    assert!(matches!(reply,R::Purges {records} if records.len()==1&&records[0].state==state));
}

pub(super) async fn refuses_replay(worker: &Worker, database: &Path) {
    for (client_id, query) in [
        (
            2,
            Q::RevisionSnapshot {
                environment_id: id(2),
                revision_id: id(3),
            },
        ),
        (3, Q::RequestSnapshot { request_id: id(40) }),
    ] {
        let (connection, grant) = worker.attach(client_id, LedgerClientRole::Reader).await;
        let client = LedgerClient::connect(connection, grant, database.to_path_buf())
            .await
            .unwrap();
        assert!(client
            .environment_read(query)
            .await
            .unwrap_err()
            .contains("non-replayable"));
        drop(client);
    }
}
