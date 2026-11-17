//! Native process/codec proof, through the real inherited control descriptor.
use super::*;

#[path = "../../src/codec/implementation.rs"]
mod native_codecs;
use capsem_config::environment::EnvironmentSpecification;
use capsem_logger::events::environment::purge::*;
use capsem_logger::events::environment::request::*;
use capsem_logger::events::{EnvironmentRevisionState, EnvironmentRevisionTransition, PreparedEnvironmentRevision};
use capsem_logger::ledger_client::LedgerClient;
use std::path::Path;

const SELECTED: &str = "native-purge-selected-value-canary";
fn id(n: u64) -> String {
    format!("00000000-0000-4000-8000-{n:012x}")
}

struct Worker {
    child: ChildGuard,
    requests: ControlSender,
    events: ControlReceiver,
    generation: LedgerGeneration,
}
impl Worker {
    async fn start(database: &Path, seed: u8) -> Self {
        let (control, child_control) = UnixStream::pair().unwrap();
        let child = ChildGuard(Some(
            Command::new(env!("CARGO_BIN_EXE_capsem-ledger"))
                .arg("--parent-pid")
                .arg(std::process::id().to_string())
                .arg("--database")
                .arg(database)
                .arg("--generation")
                .arg(format!("{seed:02x}").repeat(16))
                .stdin(Stdio::from(OwnedFd::from(child_control)))
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap(),
        ));
        let generation = LedgerGeneration::new([seed; 16]);
        let requests = ControlSender::new(control.try_clone().unwrap()).unwrap();
        let events = ControlReceiver::new(control).unwrap();
        assert_eq!(event(&events).await, LedgerControlEvent::Ready { generation });
        Self {
            child,
            requests,
            events,
            generation,
        }
    }
    async fn attach(&self, n: u64, role: LedgerClientRole) -> (UnixStream, LedgerChannelGrant) {
        let grant = LedgerChannelGrant::new(self.generation, n, role).unwrap();
        let (client, ledger) = UnixStream::pair().unwrap();
        self.requests
            .send(
                &encode_ledger_control_request(LedgerControlRequest::Attach(grant)),
                &[ledger.as_raw_fd()],
            )
            .await
            .unwrap();
        loop {
            match event(&self.events).await {
                LedgerControlEvent::Closed { .. } => (),
                adopted => {
                    assert_eq!(
                        adopted,
                        LedgerControlEvent::Adopted {
                            generation: self.generation,
                            client_id: n
                        }
                    );
                    break;
                }
            }
        }
        drop(ledger);
        (client, grant)
    }
    async fn producer(
        &self,
        n: u64,
        role: LedgerClientRole,
    ) -> (
        ipc_channel::Sender<LedgerClientMessage>,
        ipc_channel::Receiver<LedgerServerMessage>,
    ) {
        let (client, grant) = self.attach(n, role).await;
        let (sender, receiver) =
            ipc_channel::channel_from_std::<LedgerClientMessage, LedgerServerMessage>(client).unwrap();
        sender
            .send(LedgerClientMessage::Hello {
                hello: LedgerHello::for_grant(&grant),
            })
            .await
            .unwrap();
        assert!(matches!(
            receiver.recv().await.unwrap(),
            LedgerServerMessage::Welcome { .. }
        ));
        (sender, receiver)
    }
    async fn maintainer(&self, n: u64, database: &Path) -> LedgerClient {
        let (client, grant) = self.attach(n, LedgerClientRole::Maintainer).await;
        LedgerClient::connect(client, grant, database.to_path_buf())
            .await
            .unwrap()
    }
    async fn stop(mut self) {
        self.requests
            .send(
                &encode_ledger_control_request(LedgerControlRequest::Shutdown {
                    generation: self.generation,
                }),
                &[],
            )
            .await
            .unwrap();
        loop {
            match event(&self.events).await {
                LedgerControlEvent::Closed { .. } => (),
                stopped => {
                    assert_eq!(
                        stopped,
                        LedgerControlEvent::Stopped {
                            generation: self.generation
                        }
                    );
                    break;
                }
            }
        }
        let child = self.child.take();
        let output = tokio::task::spawn_blocking(move || child.wait_with_output())
            .await
            .unwrap()
            .unwrap();
        assert!(output.status.success());
    }
}

async fn command(
    sender: &ipc_channel::Sender<LedgerClientMessage>,
    receiver: &ipc_channel::Receiver<LedgerServerMessage>,
    request_id: u64,
    operation: LedgerCommand,
) -> capsem_logger::ledger_protocol::LedgerResponse {
    sender
        .send(LedgerClientMessage::Request {
            request: LedgerRequest::new(request_id, operation).unwrap(),
        })
        .await
        .unwrap();
    let LedgerServerMessage::Response { response } = tokio::time::timeout(Duration::from_secs(10), receiver.recv())
        .await
        .unwrap()
        .unwrap()
    else {
        panic!("expected response")
    };
    assert_eq!(response.request_id(), request_id);
    response
}
async fn durable(
    sender: &ipc_channel::Sender<LedgerClientMessage>,
    receiver: &ipc_channel::Receiver<LedgerServerMessage>,
    next: &mut u64,
    event: WriteOp,
) {
    for operation in [LedgerCommand::Admit { event: Box::new(event) }, LedgerCommand::Flush] {
        let response = command(sender, receiver, *next, operation).await;
        *next += 1;
        assert!(
            matches!(
                response.outcome(),
                LedgerOutcome::Success {
                    reply: LedgerReply::Accepted { .. } | LedgerReply::Durable { .. }
                }
            ),
            "{response:?}"
        );
    }
}
async fn revision(
    sender: &ipc_channel::Sender<LedgerClientMessage>,
    receiver: &ipc_channel::Receiver<LedgerServerMessage>,
    next: &mut u64,
    number: u64,
    previous: Option<u64>,
    value: Option<&str>,
) {
    let mut specification: EnvironmentSpecification = toml::from_str("name='development'").unwrap();
    specification
        .variables
        .insert("KEEP".into(), "unrelated retained field".into());
    if let Some(value) = value {
        specification.variables.insert("WRONG".into(), value.into());
    }
    durable(
        sender,
        receiver,
        next,
        WriteOp::PrepareEnvironmentRevision(PreparedEnvironmentRevision {
            environment_id: id(2),
            revision_id: id(number),
            previous_revision_id: previous.map(id),
            fork_parent: None,
            timestamp_unix_ms: number as i64,
            actor: "operator".into(),
            reason: "native fixture".into(),
            request_id: None,
            payload_event_id: format!("{number:012x}"),
            specification: Box::new(specification),
        }),
    )
    .await;
    durable(
        sender,
        receiver,
        next,
        WriteOp::EnvironmentRevisionTransition(EnvironmentRevisionTransition {
            environment_id: id(2),
            revision_id: id(number),
            state: EnvironmentRevisionState::Committed,
            timestamp_unix_ms: 20,
            actor: "operator".into(),
        }),
    )
    .await;
}

#[tokio::test]
async fn executable_variable_purge_recompresses_native_history_and_recovers_durable_results() {
    use capsem_archive::{BlockExtent, BodyLogReader, BodyRef};
    use capsem_foundation::unix::contained::ContainedDir;
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("session.db");
    let worker = Worker::start(&database, 0x71).await;
    let (sender, receiver) = worker.producer(1, LedgerClientRole::Coordinator).await;
    let mut next = 1;
    for (n, previous, value) in [
        (3, None, Some(SELECTED)),
        (4, Some(3), Some("different legitimate value")),
        (5, Some(4), Some(SELECTED)),
        (6, Some(5), None),
    ] {
        revision(&sender, &receiver, &mut next, n, previous, value).await;
    }
    durable(
        &sender,
        &receiver,
        &mut next,
        WriteOp::PrepareEnvironmentRequest(PreparedEnvironmentRequest {
            request_id: id(40),
            environment_id: id(2),
            base_revision_id: Some(id(6)),
            requester: None,
            timestamp_unix_ms: 40,
            expires_at_unix_ms: 1000,
            actor: "operator".into(),
            reason: "native proposal".into(),
            payload_event_id: "000000000028".into(),
            payload: Box::new(EnvironmentRequestPayload::Change {
                specification: Box::new(
                    toml::from_str(&format!(
                        "name='development'\n[variables]\nWRONG='{SELECTED}'\nKEEP='proposal field'"
                    ))
                    .unwrap(),
                ),
            }),
        }),
    )
    .await;
    let decision = EnvironmentRequestDecision {
        request_id: id(40),
        timestamp_unix_ms: 41,
        actor: "operator".into(),
        decision: EnvironmentRequestDecisionKind::Approve,
        reason: "original native approval".into(),
    };
    let (forger, forged_reply) = worker.producer(3, LedgerClientRole::VmOwner).await;
    for (request_id, operation) in [
        (
            1,
            LedgerCommand::Admit {
                event: Box::new(WriteOp::EnvironmentRequestDecision(decision.clone())),
            },
        ),
        (
            2,
            LedgerCommand::AdmitBatch {
                events: vec![body_event(), WriteOp::EnvironmentRequestDecision(decision.clone())],
            },
        ),
    ] {
        let response = command(&forger, &forged_reply, request_id, operation).await;
        let LedgerOutcome::Failure { failure } = response.outcome() else {
            panic!("producer forged operator approval")
        };
        assert_eq!(
            failure.code(),
            capsem_proto::ledger::LedgerFailureCode::UnauthorizedOperation
        );
    }
    durable(
        &sender,
        &receiver,
        &mut next,
        WriteOp::EnvironmentRequestDecision(decision),
    )
    .await;
    durable(
        &sender,
        &receiver,
        &mut next,
        WriteOp::PrepareEnvironmentVariablePurge(PreparedEnvironmentVariablePurge {
            purge_id: id(50),
            environment_id: id(2),
            source_revision_id: id(3),
            variable_name: "WRONG".into(),
            timestamp_unix_ms: 50,
            actor: "operator".into(),
        }),
    )
    .await;
    let client = worker.maintainer(2, &database).await;
    // Independent read-only oracle and generation lease live in the test
    // process, proving reclamation observes a different process's open reader.
    let conn = rusqlite::Connection::open_with_flags(&database, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let directory = ContainedDir::open_root(&database.with_extension("bodies")).unwrap();
    let old_name = directory
        .entries()
        .unwrap()
        .into_iter()
        .find(|entry| entry.name.to_str().is_some_and(|name| name.ends_with(".cbl")))
        .unwrap()
        .name;
    let old_path = database.with_extension("bodies").join(&old_name);
    let old_bytes = std::fs::read(&old_path).unwrap();
    assert_eq!(
        old_bytes[capsem_archive::FILE_HEADER_BYTES + 4],
        capsem_archive::CODEC_ZSTD
    );
    let header = capsem_archive::format::decode_file_header(&old_bytes[..capsem_archive::FILE_HEADER_BYTES]).unwrap();
    let old_reader = BodyLogReader::open_generation_with_codecs(
        &directory,
        header,
        old_bytes.len() as u64,
        native_codecs::archive_codecs().unwrap(),
    )
    .unwrap();
    let (offset,body_offset,len,disk_len,raw_len):(i64,i64,i64,i64,i64)=conn.query_row("SELECT b.block_offset,b.body_offset,b.body_len,k.disk_len,k.raw_len FROM event_body_blobs b JOIN body_blocks k ON k.block_offset=b.block_offset WHERE b.event_id='000000000003' AND b.source_table='environment_revisions'",[],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?))).unwrap();
    let execution = EnvironmentVariablePurgeExecution {
        purge_id: id(50),
        timestamp_unix_ms: 51,
    };
    let pending = client.purge_variable_history(execution.clone()).await.unwrap();
    assert_eq!(pending.state, EnvironmentVariablePurgeState::ReclaimPending);
    assert!(old_path.exists());
    // Restart while another process still leases the old generation. Recovery
    // must preserve the published generation and the outstanding reclamation.
    drop(client);
    drop(sender);
    drop(receiver);
    drop(forger);
    drop(forged_reply);
    worker.stop().await;
    let worker = Worker::start(&database, 0x72).await;
    let client = worker.maintainer(1, &database).await;
    let resumed = client.purge_variable_history(execution.clone()).await.unwrap();
    assert_eq!(resumed.state, EnvironmentVariablePurgeState::ReclaimPending);
    assert_eq!(resumed.new_generation_id, pending.new_generation_id);
    assert!(old_path.exists());
    let captured = old_reader
        .read_bounded(
            BodyRef {
                block_offset: offset as u64,
                offset: body_offset as u32,
                len: len as u32,
            },
            BlockExtent {
                disk_len: disk_len as u64,
                raw_len: raw_len as u32,
            },
        )
        .unwrap();
    assert_eq!(
        rmp_serde::from_slice::<EnvironmentSpecification>(&captured)
            .unwrap()
            .variables["WRONG"],
        SELECTED
    );
    drop(old_reader);
    let complete = client.purge_variable_history(execution.clone()).await.unwrap();
    assert_eq!(complete.state, EnvironmentVariablePurgeState::Committed);
    assert!(!old_path.exists());
    assert_eq!(complete.new_generation_id, pending.new_generation_id);
    for n in [3, 5] {
        let bodies = client.read_bodies(&format!("{n:012x}")).await.unwrap();
        assert_eq!(bodies.len(), 1);
        let spec: EnvironmentSpecification = rmp_serde::from_slice(&bodies[0].bytes).unwrap();
        assert!(!spec.variables.contains_key("WRONG"));
        assert_eq!(spec.variables["KEEP"], "unrelated retained field");
        assert!(!conn
            .query_row(
                "SELECT replayable FROM environment_revisions WHERE revision_id=?1",
                [id(n)],
                |row| row.get::<_, bool>(0)
            )
            .unwrap());
    }
    let untouched = client.read_bodies("000000000004").await.unwrap();
    assert_eq!(
        rmp_serde::from_slice::<EnvironmentSpecification>(&untouched[0].bytes)
            .unwrap()
            .variables["WRONG"],
        "different legitimate value"
    );
    let proposal = client.read_bodies("000000000028").await.unwrap();
    let payload: EnvironmentRequestPayload = rmp_serde::from_slice(&proposal[0].bytes).unwrap();
    let EnvironmentRequestPayload::Change { specification } = payload else {
        panic!("proposal")
    };
    assert!(!specification.variables.contains_key("WRONG"));
    assert_eq!(specification.variables["KEEP"], "proposal field");
    assert_eq!(conn.query_row("SELECT q.state,d.reason FROM environment_requests q JOIN environment_request_decisions d ON d.request_id=q.request_id WHERE q.request_id=?1",[id(40)],|row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?))).unwrap(),("stale".into(),"original native approval".into()));
    let new_id = capsem_archive::GenerationId::from_bytes(complete.new_generation_id.unwrap()).unwrap();
    let new_path = database.with_extension("bodies").join(new_id.file_name());
    let new_bytes = std::fs::read(&new_path).unwrap();
    assert_eq!(
        new_bytes[capsem_archive::FILE_HEADER_BYTES + 4],
        capsem_archive::CODEC_ZSTD
    );
    let header = capsem_archive::format::decode_file_header(&new_bytes[..capsem_archive::FILE_HEADER_BYTES]).unwrap();
    let fresh = BodyLogReader::open_generation_with_codecs(
        &directory,
        header,
        new_bytes.len() as u64,
        native_codecs::archive_codecs().unwrap(),
    )
    .unwrap();
    let mut statement = conn
        .prepare("SELECT block_offset,disk_len,raw_len FROM body_blocks ORDER BY block_offset")
        .unwrap();
    let blocks = statement
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?))
        })
        .unwrap();
    for block in blocks {
        let (offset, disk_len, raw_len) = block.unwrap();
        let bytes = fresh
            .read_bounded(
                BodyRef {
                    block_offset: offset as u64,
                    offset: 0,
                    len: raw_len as u32,
                },
                BlockExtent {
                    disk_len: disk_len as u64,
                    raw_len: raw_len as u32,
                },
            )
            .unwrap();
        assert!(
            !bytes
                .windows(SELECTED.len())
                .any(|window| window == SELECTED.as_bytes()),
            "native compression dictionary retained selected bytes"
        );
    }
    drop(statement);
    drop(fresh);
    drop(conn);
    drop(client);
    worker.stop().await;
    let restarted = Worker::start(&database, 0x73).await;
    let client = restarted.maintainer(1, &database).await;
    assert_eq!(client.purge_variable_history(execution).await.unwrap(), complete);
    assert_eq!(client.read_bodies("000000000003").await.unwrap().len(), 1);
    drop(client);
    restarted.stop().await;
}
