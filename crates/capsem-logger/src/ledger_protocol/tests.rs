use std::os::unix::net::UnixStream;
use std::time::SystemTime;

use capsem_foundation::ipc_channel;
use capsem_proto::ledger::{LedgerChannelGrant, LedgerClientRole, LedgerGeneration, LedgerRequest};
use capsem_proto::ledger_commitment::{LedgerCommitment, ZERO_COMMITMENT_HASH};

use super::*;
use crate::{ExecEventComplete, FileAction, FileEvent, FileKind, WriteOp};

fn grant(role: LedgerClientRole) -> LedgerChannelGrant {
    LedgerChannelGrant::new(LedgerGeneration::new([3; 16]), 9, role).unwrap()
}

fn file_write() -> LedgerCommand {
    let event = WriteOp::FileEvent(FileEvent {
        event_id: None,
        timestamp: SystemTime::UNIX_EPOCH,
        action: FileAction::Modified,
        path: "/root/report.txt".into(),
        size: Some(12),
        kind: FileKind::File,
        trace_id: None,
        credential_ref: None,
    });
    LedgerCommand::Admit {
        commitment: commitment(&event),
        event: Box::new(event),
    }
}

fn commitment(event: &WriteOp) -> LedgerCommitment {
    LedgerCommitment::new(
        grant(LedgerClientRole::VmOwner),
        1,
        1,
        event.kind(),
        crate::writer::commitment_event_hash(event).unwrap(),
        ZERO_COMMITMENT_HASH,
    )
    .unwrap()
}

#[tokio::test]
async fn request_round_trips_over_the_landed_bounded_channel() {
    let request = LedgerRequest::new(1, file_write()).unwrap();
    request.authorize(&grant(LedgerClientRole::VmOwner)).unwrap();
    let (left, right) = UnixStream::pair().unwrap();
    let (sender, _) = ipc_channel::channel_from_std::<LedgerRequest<LedgerCommand>, LedgerResponse>(left).unwrap();
    let (_, receiver) = ipc_channel::channel_from_std::<LedgerResponse, LedgerRequest<LedgerCommand>>(right).unwrap();
    sender.send(request).await.unwrap();
    let decoded = receiver.recv().await.unwrap();
    assert_eq!(decoded.request_id(), 1);
    assert!(matches!(
        decoded.operation(),
        LedgerCommand::Admit {
            event, ..
        } if matches!(event.as_ref(), WriteOp::FileEvent(event) if event.path == "/root/report.txt")
    ));
}

#[test]
fn admission_batch_is_bounded_and_correlated_as_one_reply() {
    let LedgerCommand::Admit { event, commitment } = file_write() else {
        unreachable!()
    };
    let admission = LedgerAdmission::new(*event, commitment);
    LedgerCommand::AdmitBatch {
        admissions: vec![admission.clone(), admission],
    }
    .validate()
    .unwrap();
    assert_eq!(
        LedgerCommand::AdmitBatch { admissions: vec![] }.validate(),
        Err(LedgerProtocolError::InvalidOperation)
    );
    LedgerReply::AcceptedBatch {
        first_admission_id: 41,
        through_admission_id: 42,
        accepted: 2,
    }
    .validate()
    .unwrap();
    assert!(LedgerReply::AcceptedBatch {
        first_admission_id: 41,
        through_admission_id: 43,
        accepted: 2,
    }
    .validate()
    .is_err());
}

#[test]
fn roles_admit_only_their_operation_classes() {
    for (role, operation) in [
        (LedgerClientRole::VmOwner, file_write()),
        (LedgerClientRole::Reader, LedgerCommand::Counters),
        (
            LedgerClientRole::Maintainer,
            LedgerCommand::Retain {
                cutoff: "2026-10-09T00:00:00.000000Z".into(),
            },
        ),
        (LedgerClientRole::Reader, LedgerCommand::ExportWarc),
        (
            LedgerClientRole::Maintainer,
            LedgerCommand::Snapshot { snapshot_id: [1; 16] },
        ),
        (LedgerClientRole::Supervisor, LedgerCommand::Shutdown),
    ] {
        LedgerRequest::new(1, operation)
            .unwrap()
            .authorize(&grant(role))
            .unwrap();
    }
}

#[test]
fn snapshot_ids_must_be_nonzero_fixed_width_values() {
    assert_eq!(
        LedgerCommand::Snapshot { snapshot_id: [0; 16] }.validate(),
        Err(LedgerProtocolError::InvalidOperation)
    );
    LedgerCommand::Snapshot { snapshot_id: [1; 16] }.validate().unwrap();
}

#[test]
fn oversized_event_and_malformed_query_are_rejected_before_dispatch() {
    let huge_event = WriteOp::ExecEventComplete(ExecEventComplete {
        exec_id: 1,
        exit_code: 0,
        duration_ms: 1,
        stdout: vec![b'x'; MAX_LEDGER_OPERATION_BYTES],
        stderr: Vec::new(),
        stdout_bytes: MAX_LEDGER_OPERATION_BYTES as u64,
        stderr_bytes: 0,
        pid: None,
    });
    let huge = LedgerCommand::Admit {
        commitment: commitment(&huge_event),
        event: Box::new(huge_event),
    };
    assert_eq!(
        LedgerRequest::new(1, huge)
            .unwrap()
            .authorize(&grant(LedgerClientRole::VmOwner))
            .unwrap_err(),
        capsem_proto::ledger::LedgerProtocolError::OperationTooLarge
    );
    let empty_timeline = LedgerCommand::Query {
        query: LedgerQuery::Timeline {
            layers: Vec::new(),
            cutoff: String::new(),
            trace_id: None,
            limit: 10,
        },
    };
    assert_eq!(
        LedgerRequest::new(2, empty_timeline)
            .unwrap()
            .authorize(&grant(LedgerClientRole::Reader))
            .unwrap_err(),
        capsem_proto::ledger::LedgerProtocolError::InvalidOperation
    );
}

#[test]
fn admission_metadata_stays_within_its_reserved_frame_allowance() {
    let command = file_write();
    let LedgerCommand::Admit { event, .. } = &command else {
        unreachable!("file_write always returns an admission")
    };
    let event_bytes = serde_json::to_vec(event).unwrap().len();
    let operation_bytes = serde_json::to_vec(&command).unwrap().len();

    assert!(operation_bytes - event_bytes < MAX_ADMIT_METADATA_BYTES);
}

#[test]
fn event_bytes_use_messagepack_binary_inside_the_request() {
    let event = WriteOp::ExecEventComplete(ExecEventComplete {
        exec_id: 1,
        exit_code: 0,
        duration_ms: 1,
        stdout: vec![0xa5; 1024 * 1024],
        stderr: Vec::new(),
        stdout_bytes: 1024 * 1024,
        stderr_bytes: 0,
        pid: None,
    });
    let command = LedgerCommand::Admit {
        commitment: commitment(&event),
        event: Box::new(event),
    };
    let encoded = rmp_serde::to_vec_named(&LedgerRequest::new(1, command).unwrap()).unwrap();
    assert!(
        encoded.len() < 1024 * 1024 + 1024,
        "binary payload expanded to {}",
        encoded.len()
    );
}

#[test]
fn streamed_replies_enforce_chunk_and_row_bounds() {
    assert!(LedgerReply::WarcChunk {
        offset: 0,
        bytes: vec![0; MAX_LEDGER_STREAM_CHUNK_BYTES],
    }
    .validate()
    .is_ok());
    assert!(LedgerReply::WarcChunk {
        offset: 0,
        bytes: vec![0; MAX_LEDGER_STREAM_CHUNK_BYTES + 1],
    }
    .validate()
    .is_err());
    assert!(LedgerRows {
        columns: vec!["one".into()],
        rows: vec![vec![]],
    }
    .validate()
    .is_err());
}

#[test]
fn table_rows_query_validates_table_allowlist_cursor_and_limit() {
    assert_eq!(DEFAULT_EXPORT_LEDGER_TABLES.len(), 16);
    assert_eq!(EXPORTABLE_LEDGER_TABLES.len(), 18);
    assert!(!DEFAULT_EXPORT_LEDGER_TABLES.contains(&"security_rule_events"));
    assert!(!DEFAULT_EXPORT_LEDGER_TABLES.contains(&"security_decision_events"));
    assert!(EXPORTABLE_LEDGER_TABLES.contains(&"security_rule_events"));
    assert!(EXPORTABLE_LEDGER_TABLES.contains(&"security_decision_events"));

    assert!(LedgerQuery::TableWatermarks {
        tables: EXPORTABLE_LEDGER_TABLES.iter().map(|t| (*t).to_string()).collect(),
    }
    .validate()
    .is_ok());

    for table in EXPORTABLE_LEDGER_TABLES {
        assert!(LedgerQuery::TableRows {
            table: (*table).to_string(),
            after_id: 0,
            max_id: Some(100),
            limit: 2_000,
        }
        .validate()
        .is_ok());
    }
    for invalid in [
        LedgerQuery::TableWatermarks { tables: Vec::new() },
        LedgerQuery::TableWatermarks {
            tables: vec!["sqlite_master".to_string()],
        },
        LedgerQuery::TableRows {
            table: "sqlite_master".to_string(),
            after_id: 0,
            max_id: None,
            limit: 100,
        },
        LedgerQuery::TableRows {
            table: "ledger_commitments".to_string(),
            after_id: 0,
            max_id: None,
            limit: 100,
        },
        LedgerQuery::TableRows {
            table: "model_calls".to_string(),
            after_id: -1,
            max_id: None,
            limit: 100,
        },
        LedgerQuery::TableRows {
            table: "model_calls".to_string(),
            after_id: 5,
            max_id: Some(4),
            limit: 100,
        },
        LedgerQuery::TableRows {
            table: "model_calls".to_string(),
            after_id: 0,
            max_id: None,
            limit: 0,
        },
        LedgerQuery::TableRows {
            table: "model_calls".to_string(),
            after_id: 0,
            max_id: None,
            limit: 2_001,
        },
    ] {
        assert_eq!(invalid.validate(), Err(LedgerProtocolError::InvalidOperation));
    }
}
