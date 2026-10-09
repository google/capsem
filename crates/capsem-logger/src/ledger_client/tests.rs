use std::sync::Arc;
use std::time::SystemTime;

use capsem_proto::ledger::{LedgerChannelGrant, LedgerClientRole, LedgerGeneration};

use super::*;
use crate::events::{Decision, NetEvent};
use crate::ledger_protocol::LedgerQuery;
use crate::{ledger_server::LedgerServer, WriteOp};

const GENERATION: LedgerGeneration = LedgerGeneration::new([0x51; 16]);
const EVENT_ID: &str = "0123456789ab";

struct Fixture {
    _dir: tempfile::TempDir,
    client: LedgerClient,
    writer: crate::DbWriter,
    producer_task: tokio::task::JoinHandle<Result<crate::ledger_server::LedgerClientExit, String>>,
    reader_task: tokio::task::JoinHandle<Result<crate::ledger_server::LedgerClientExit, String>>,
}

impl Fixture {
    async fn start(role: LedgerClientRole) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.db");
        let server = Arc::new(LedgerServer::open(&path).unwrap());

        let producer_grant = LedgerChannelGrant::new(GENERATION, 1, LedgerClientRole::VmOwner).unwrap();
        let (producer, producer_server) = std::os::unix::net::UnixStream::pair().unwrap();
        let producer_owner = Arc::clone(&server);
        let producer_task =
            tokio::spawn(async move { producer_owner.serve_client(producer_server, producer_grant).await });
        let writer = tokio::task::spawn_blocking(move || {
            crate::DbWriter::from_ledger_channel(producer, producer_grant, &path, 16)
        })
        .await
        .unwrap()
        .unwrap();

        let reader_grant = LedgerChannelGrant::new(GENERATION, 2, role).unwrap();
        let (reader, reader_server) = std::os::unix::net::UnixStream::pair().unwrap();
        let reader_owner = Arc::clone(&server);
        let reader_task = tokio::spawn(async move { reader_owner.serve_client(reader_server, reader_grant).await });
        let client = LedgerClient::connect(reader, reader_grant, dir.path().join("session.db"))
            .await
            .unwrap();
        Self {
            _dir: dir,
            client,
            writer,
            producer_task,
            reader_task,
        }
    }

    async fn write_network_body(&self) {
        self.writer
            .write_checked(WriteOp::NetEvent(NetEvent {
                event_id: Some(EVENT_ID.into()),
                timestamp: SystemTime::now(),
                domain: "example.test".into(),
                port: 443,
                decision: Decision::Allowed,
                process_name: Some("curl".into()),
                pid: Some(7),
                method: Some("POST".into()),
                path: Some("/v1".into()),
                query: None,
                status_code: Some(200),
                bytes_sent: 7,
                bytes_received: 8,
                duration_ms: 9,
                matched_rule: None,
                request_headers: None,
                response_headers: None,
                request_body: Some(b"request".to_vec()),
                response_body: Some(b"response".to_vec()),
                conn_type: Some("tls".into()),
                policy_mode: None,
                policy_action: None,
                policy_rule: None,
                policy_reason: None,
                trace_id: None,
                credential_ref: None,
            }))
            .await
            .unwrap();
        self.writer.flush_checked().await.unwrap();
    }

    async fn stop(self) {
        let Fixture {
            client,
            writer,
            producer_task,
            reader_task,
            ..
        } = self;
        drop(client);
        tokio::task::spawn_blocking(move || writer.shutdown_blocking())
            .await
            .unwrap();
        assert_eq!(
            producer_task.await.unwrap().unwrap(),
            crate::ledger_server::LedgerClientExit::Disconnected
        );
        assert_eq!(
            reader_task.await.unwrap().unwrap(),
            crate::ledger_server::LedgerClientExit::Disconnected
        );
    }
}

#[tokio::test]
async fn reader_queries_counters_bodies_and_warc_through_one_serial_channel() {
    let fixture = Fixture::start(LedgerClientRole::Reader).await;
    assert_eq!(fixture.client.path(), fixture._dir.path().join("session.db"));
    assert_eq!(fixture.client.role(), LedgerClientRole::Reader);
    fixture.write_network_body().await;

    let counters = fixture.client.counters().await.unwrap();
    assert_eq!(counters.net.total, 1);
    assert_eq!(fixture.client.read_cache_epoch(), 1);
    assert_eq!(fixture.client.counters().await.unwrap().net.total, 1);
    assert_eq!(fixture.client.read_cache_epoch(), 1);

    let sets = fixture.client.query(LedgerQuery::StatsDetail).await.unwrap();
    assert_eq!(sets.len(), 11);
    let bodies = fixture.client.read_bodies(EVENT_ID).await.unwrap();
    assert_eq!(bodies.len(), 2);
    assert_eq!(bodies[0].bytes, b"request");
    assert_eq!(bodies[1].bytes, b"response");

    let mut export = fixture.client.export_warc().await.unwrap();
    let mut warc = Vec::new();
    while let Some(chunk) = export.next_chunk().await {
        warc.extend_from_slice(&chunk.unwrap());
    }
    let summary = export.finish().await.unwrap();
    assert_eq!(summary.records, 2);
    assert_eq!(summary.bytes_written, warc.len() as u64);
    assert!(!warc.is_empty());
    fixture.stop().await;
}

#[tokio::test]
async fn counters_advance_the_cache_epoch_only_when_the_snapshot_changes() {
    let fixture = Fixture::start(LedgerClientRole::Reader).await;
    assert_eq!(fixture.client.counters().await.unwrap().net.total, 0);
    assert_eq!(fixture.client.read_cache_epoch(), 1);
    fixture.write_network_body().await;
    assert_eq!(fixture.client.counters().await.unwrap().net.total, 1);
    assert_eq!(fixture.client.read_cache_epoch(), 2);
    fixture.stop().await;
}

#[tokio::test]
async fn server_refusal_is_reported_and_closes_the_failed_client_actor() {
    let fixture = Fixture::start(LedgerClientRole::Reader).await;
    let error = fixture
        .client
        .retain_bodies_since("2026-01-01T00:00:00.000000Z")
        .await
        .unwrap_err();
    assert!(error.contains("UnauthorizedOperation"), "{error}");
    let stopped = fixture.client.counters().await.unwrap_err();
    assert!(stopped.contains("stopped") || stopped.contains("closed"), "{stopped}");
    fixture.stop().await;
}
