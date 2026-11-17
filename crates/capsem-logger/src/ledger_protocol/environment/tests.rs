use super::*;
use crate::events::environment::purge::EnvironmentVariablePurgeState;
use crate::events::{EnvironmentRevisionParent, EnvironmentRevisionState};
use capsem_proto::ledger::{LedgerChannelGrant, LedgerClientRole, LedgerGeneration};

fn id(n: u64) -> String {
    format!("00000000-0000-4000-8000-{n:012x}")
}
fn record() -> EnvironmentRevisionRecord {
    EnvironmentRevisionRecord {
        cursor: 1,
        environment_id: id(2),
        revision_id: id(3),
        previous_revision_id: None,
        fork_parent: None,
        timestamp_unix_ms: 1,
        actor: "operator".into(),
        reason: "review".into(),
        request_id: None,
        state: EnvironmentRevisionState::Committed,
        payload_event_id: "000000000003".into(),
        replayable: true,
    }
}

#[test]
fn environment_reads_require_read_authority_and_validate_every_query_family() {
    let query = LedgerEnvironmentQuery::Revisions {
        query: EnvironmentRevisionQuery::Get {
            environment_id: id(2),
            revision_id: id(3),
        },
    };
    for role in [
        LedgerClientRole::VmOwner,
        LedgerClientRole::Proxy,
        LedgerClientRole::Coordinator,
        LedgerClientRole::Reader,
        LedgerClientRole::Maintainer,
        LedgerClientRole::Supervisor,
    ] {
        let grant = LedgerChannelGrant::new(LedgerGeneration::new([1; 16]), 1, role).unwrap();
        assert_eq!(
            LedgerRequest::new(1, LedgerCommand::EnvironmentRead { query: query.clone() })
                .unwrap()
                .authorize(&grant)
                .is_ok(),
            matches!(role, LedgerClientRole::Reader | LedgerClientRole::Maintainer)
        );
    }
    for query in [
        LedgerEnvironmentQuery::Revisions {
            query: EnvironmentRevisionQuery::Page {
                environment_id: None,
                state: None,
                after_id: 0,
                limit: 0,
            },
        },
        LedgerEnvironmentQuery::RevisionSnapshot {
            environment_id: "bad".into(),
            revision_id: id(3),
        },
        LedgerEnvironmentQuery::RevisionHistory {
            environment_id: id(2),
            revision_id: "bad".into(),
        },
        LedgerEnvironmentQuery::Requests {
            query: EnvironmentRequestQuery::Page {
                environment_id: None,
                requester: None,
                pending_only: false,
                after_id: -1,
                limit: 1,
            },
        },
        LedgerEnvironmentQuery::RequestSnapshot {
            request_id: "bad".into(),
        },
        LedgerEnvironmentQuery::RequestHistory {
            request_id: "bad".into(),
        },
        LedgerEnvironmentQuery::References {
            query: EnvironmentReferenceQuery::Sandbox {
                session_id: id(2),
                generation: "bad".into(),
            },
        },
        LedgerEnvironmentQuery::Applications {
            query: EnvironmentApplicationQuery::Page {
                request_id: None,
                target: None,
                after_id: 0,
                limit: 2001,
            },
        },
        LedgerEnvironmentQuery::Purges {
            query: EnvironmentVariablePurgeQuery::Get { purge_id: "bad".into() },
        },
    ] {
        assert!(query.validate().is_err());
    }
}

#[test]
fn environment_replies_enforce_scope_cursor_cardinality_and_marker_consistency() {
    let query = LedgerEnvironmentQuery::Revisions {
        query: EnvironmentRevisionQuery::Get {
            environment_id: id(2),
            revision_id: id(3),
        },
    };
    let valid = LedgerEnvironmentReply::Revisions {
        records: vec![record()],
    };
    valid.validate().unwrap();
    assert!(valid.matches_query(&query));
    let mut wrong = record();
    wrong.environment_id = id(9);
    let wrong = LedgerEnvironmentReply::Revisions { records: vec![wrong] };
    wrong.validate().unwrap();
    assert!(!wrong.matches_query(&query));
    assert!(!LedgerEnvironmentReply::Requests { records: vec![] }.matches_query(&query));
    let page = LedgerEnvironmentQuery::Revisions {
        query: EnvironmentRevisionQuery::Page {
            environment_id: Some(id(2)),
            state: None,
            after_id: 0,
            limit: 1,
        },
    };
    assert!(!LedgerEnvironmentReply::Revisions {
        records: vec![record(), record()]
    }
    .matches_query(&page));
    let mut later = record();
    later.cursor = 2;
    let page = LedgerEnvironmentQuery::Revisions {
        query: EnvironmentRevisionQuery::Page {
            environment_id: None,
            state: None,
            after_id: 0,
            limit: 2,
        },
    };
    assert!(!LedgerEnvironmentReply::Revisions {
        records: vec![later, record()]
    }
    .matches_query(&page));
    assert!(LedgerEnvironmentReply::Revisions {
        records: vec![record(); MAX_LEDGER_QUERY_ROWS + 1]
    }
    .validate()
    .is_err());
    let snapshot = EnvironmentRevisionSnapshot {
        record: record(),
        specification: Box::new(toml::from_str("name='development'").unwrap()),
    };
    let frame = LedgerEnvironmentReply::RevisionSnapshot {
        snapshot: Some(Box::new(snapshot.clone())),
    };
    let encoded = rmp_serde::to_vec_named(&frame).unwrap();
    let decoded: LedgerEnvironmentReply = rmp_serde::from_slice(&encoded).unwrap();
    assert_eq!(decoded, frame);
    decoded.validate().unwrap();
    let mut nonreplay = snapshot.record.clone();
    nonreplay.replayable = false;
    assert!(LedgerEnvironmentReply::RevisionSnapshot {
        snapshot: Some(Box::new(EnvironmentRevisionSnapshot {
            record: nonreplay.clone(),
            ..snapshot.clone()
        }))
    }
    .validate()
    .is_err());
    let marker = crate::db::EnvironmentVariablePurgeMarker {
        purge_id: id(5),
        variable_name: "WRONG".into(),
        timestamp_unix_ms: 10,
        state: EnvironmentVariablePurgeState::Committed,
    };
    let partial = EnvironmentRevisionHistory {
        record: nonreplay,
        payload: EnvironmentHistoryPayload::Purged {
            value: snapshot.specification,
            purged_variables: vec![marker],
        },
    };
    let valid = LedgerEnvironmentReply::RevisionHistory {
        history: Some(Box::new(partial.clone())),
    };
    valid.validate().unwrap();
    let mut wrong = partial.clone();
    wrong.record.replayable = true;
    assert!(LedgerEnvironmentReply::RevisionHistory {
        history: Some(Box::new(wrong))
    }
    .validate()
    .is_err());
    let mut wrong = partial;
    let EnvironmentHistoryPayload::Purged { purged_variables, .. } = &mut wrong.payload else {
        unreachable!()
    };
    purged_variables[0].state = EnvironmentVariablePurgeState::Prepared;
    assert!(LedgerEnvironmentReply::RevisionHistory {
        history: Some(Box::new(wrong.clone()))
    }
    .validate()
    .is_err());
    let EnvironmentHistoryPayload::Purged {
        value,
        purged_variables,
    } = &mut wrong.payload
    else {
        unreachable!()
    };
    purged_variables[0].state = EnvironmentVariablePurgeState::Committed;
    value.variables.insert("WRONG".into(), "must not survive".into());
    assert!(LedgerEnvironmentReply::RevisionHistory {
        history: Some(Box::new(wrong))
    }
    .validate()
    .is_err());
    assert!(LedgerEnvironmentReply::References {
        records: vec![EnvironmentReferenceRecord::CurrentEnvironment {
            revision: EnvironmentRevisionParent {
                environment_id: id(2),
                revision_id: "bad".into()
            }
        }]
    }
    .validate()
    .is_err());
}
