use super::*;
use crate::events::{MembershipState, NetworkMembership, NetworkRecord, TransportEvent, TransportEventKind};
use crate::{DbHandle, WriteOp};
use std::net::Ipv4Addr;
use uuid::Uuid;

fn network_at(dir: &tempfile::TempDir) -> std::path::PathBuf {
    dir.path().join("networks").join("n1").join("network.db")
}

const SUBNET: (Ipv4Addr, u8) = (Ipv4Addr::new(10, 128, 7, 0), 24);

#[tokio::test]
async fn network_departure_is_atomic_when_network_retirement_write_fails() {
    let directory = tempfile::tempdir().unwrap();
    let path = network_at(&directory);
    let id = capsem_crypto::uuid_v4();
    let record = NetworkRecord::new(id, "atomic", SUBNET, 1000).unwrap();
    let membership =
        NetworkMembership::new(id, "vm", Ipv4Addr::new(10, 128, 7, 2), MembershipState::Declared, 1001).unwrap();
    let db = open(&path).unwrap();
    db.write(WriteOp::Network(record.clone())).await.unwrap();
    db.write(WriteOp::NetworkMembership(membership)).await.unwrap();
    db.flush().await.unwrap();
    drop(db);
    // Configure the real database before its sole writer is reopened.
    Connection::open(&path)
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER reject_retire BEFORE UPDATE ON network WHEN NEW.state = 'retired'
         BEGIN SELECT RAISE(ABORT, 'fixture retirement failure'); END;",
        )
        .unwrap();
    let departure = crate::NetworkDeparture::new(
        NetworkMembership::new(id, "vm", Ipv4Addr::new(10, 128, 7, 2), MembershipState::Detached, 1002).unwrap(),
        Some(record.retired(1002).unwrap()),
    )
    .unwrap();
    let db = open(&path).unwrap();
    let accepted = db.write(WriteOp::NetworkDeparture(departure.clone())).await;
    let flushed = db.flush().await;
    assert!(
        accepted.is_err() || flushed.is_err(),
        "the real retirement trigger must fail"
    );
    let rows = db.query("SELECT network.state, network_members.state FROM network JOIN network_members ON network.id = network_members.network_id", &[]).await.unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&rows).unwrap()["rows"][0],
        serde_json::json!(["active", "declared"])
    );
    drop(db);
    Connection::open(&path)
        .unwrap()
        .execute_batch("DROP TRIGGER reject_retire")
        .unwrap();
    let db = open(&path).unwrap();
    db.write(WriteOp::NetworkDeparture(departure)).await.unwrap();
    db.flush().await.unwrap();
    let rows = db.query("SELECT network.state, network_members.state FROM network JOIN network_members ON network.id = network_members.network_id", &[]).await.unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&rows).unwrap()["rows"][0],
        serde_json::json!(["retired", "detached"])
    );
}

#[tokio::test]
async fn opening_creates_the_schema_and_reopening_keeps_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = network_at(&dir);
    let db = open(&path).expect("first open creates the database and its parents");
    db.ready().await.unwrap();
    drop(db);
    let db = open(&path).expect("reopen validates the marker");
    let version = db
        .query("SELECT version FROM network_schema WHERE id = 1", &[])
        .await
        .unwrap();
    assert!(version.contains(&NETWORK_SCHEMA_VERSION.to_string()), "{version}");
}

#[tokio::test]
async fn a_missing_network_table_is_a_broken_database_not_an_empty_one() {
    let dir = tempfile::tempdir().unwrap();
    let path = network_at(&dir);
    drop(open(&path).unwrap());
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute_batch("DROP TABLE network_members")
        .unwrap();
    let error = open(&path).err().expect("a dropped table must refuse to open");
    assert!(error.to_string().contains("network_members"), "{error}");
}

#[tokio::test]
async fn a_network_table_without_a_subnet_is_a_broken_database() {
    let dir = tempfile::tempdir().unwrap();
    let path = network_at(&dir);
    drop(open(&path).unwrap());
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute_batch("ALTER TABLE network DROP COLUMN subnet")
        .unwrap();
    let error = open(&path)
        .err()
        .expect("a network without its subnet must refuse to open");
    assert!(error.to_string().contains("subnet"), "{error}");
}

#[tokio::test]
async fn an_unsupported_schema_version_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = network_at(&dir);
    drop(open(&path).unwrap());
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute_batch("UPDATE network_schema SET version = 99")
        .unwrap();
    let error = open(&path)
        .err()
        .expect("a future version must not be read as the current one");
    assert!(error.to_string().contains("99"), "{error}");
}

#[tokio::test]
async fn records_are_durable_after_a_flush_barrier_and_upserted_by_key() {
    let dir = tempfile::tempdir().unwrap();
    let path = network_at(&dir);
    let db = open(&path).unwrap();
    let id = capsem_crypto::uuid_v4();
    let network = NetworkRecord::new(id, "backend", SUBNET, 1_700_000_000_000).unwrap();
    db.write(WriteOp::Network(network.clone())).await.unwrap();
    let member = |state, at| NetworkMembership::new(id, "vm-1", Ipv4Addr::new(10, 128, 0, 2), state, at).unwrap();
    db.write(WriteOp::NetworkMembership(member(
        MembershipState::Attaching,
        1_700_000_000_001,
    )))
    .await
    .unwrap();
    db.flush().await.unwrap();

    // What another process sees on disk after the barrier.
    let reader = DbHandle::open_external_reader(&path).unwrap();
    let networks = reader
        .query("SELECT name, subnet, state FROM network", &[])
        .await
        .unwrap();
    assert!(
        networks.contains("backend") && networks.contains("10.128.7.0/24") && networks.contains("active"),
        "{networks}"
    );
    let members = reader
        .query("SELECT vm_id, address, state FROM network_members", &[])
        .await
        .unwrap();
    assert!(
        members.contains("vm-1") && members.contains("10.128.0.2") && members.contains("attaching"),
        "{members}"
    );

    // A later state is the same row, not a second membership.
    db.write(WriteOp::NetworkMembership(member(
        MembershipState::Ready,
        1_700_000_000_002,
    )))
    .await
    .unwrap();
    db.write(WriteOp::Network(network.retired(1_700_000_000_003).unwrap()))
        .await
        .unwrap();
    db.flush().await.unwrap();
    let members = reader
        .query(
            "SELECT COUNT(*), MAX(state), MAX(updated_unix_ms) FROM network_members",
            &[],
        )
        .await
        .unwrap();
    assert!(members.contains("[1,\"ready\",1700000000002]"), "{members}");
    let networks = reader
        .query(
            "SELECT COUNT(*), MAX(state), MAX(retired_unix_ms), MAX(subnet) FROM network",
            &[],
        )
        .await
        .unwrap();
    assert!(
        networks.contains("[1,\"retired\",1700000000003,\"10.128.7.0/24\"]"),
        "{networks}"
    );
}

#[tokio::test]
async fn audit_rows_for_a_network_use_the_shared_transport_ledger() {
    let dir = tempfile::tempdir().unwrap();
    let path = network_at(&dir);
    let db = open(&path).unwrap();
    let id = capsem_crypto::uuid_v4();
    let row = TransportEvent::new(
        "abcdef123456".into(),
        1_700_000_000_000,
        TransportEventKind::Lifecycle,
        Some(id),
        None,
        &serde_json::json!({"action": "created"}),
    )
    .unwrap();
    db.write(WriteOp::TransportEvent(row)).await.unwrap();
    db.flush().await.unwrap();
    let rows = db
        .query(
            "SELECT event_type FROM transport_events WHERE network_id = ?1",
            &[serde_json::Value::String(id.to_string())],
        )
        .await
        .unwrap();
    assert!(rows.contains("network.lifecycle"), "{rows}");
}

#[test]
fn records_validate_identity_and_bounds_before_they_reach_sqlite() {
    assert!(NetworkRecord::new(Uuid::nil(), "backend", SUBNET, 0).is_err());
    assert!(NetworkRecord::new(capsem_crypto::uuid_v4(), "", SUBNET, 0).is_err());
    assert!(NetworkRecord::new(capsem_crypto::uuid_v4(), &"n".repeat(65), SUBNET, 0).is_err());
    assert!(NetworkRecord::new(capsem_crypto::uuid_v4(), "two words", SUBNET, 0).is_err());
    assert!(NetworkRecord::new(capsem_crypto::uuid_v4(), "backend", SUBNET, -1).is_err());
    for subnet in [
        (Ipv4Addr::new(10, 128, 7, 1), 24),
        (Ipv4Addr::new(10, 128, 7, 0), 7),
        (Ipv4Addr::new(10, 128, 7, 0), 31),
        (Ipv4Addr::new(10, 128, 7, 0), 33),
    ] {
        assert!(
            NetworkRecord::new(capsem_crypto::uuid_v4(), "backend", subnet, 0).is_err(),
            "{subnet:?}"
        );
    }
    let network = NetworkRecord::new(capsem_crypto::uuid_v4(), "backend", SUBNET, 10).unwrap();
    assert!(network.clone().retired(9).is_err());
    assert!(network.retired(10).is_ok());

    let id = capsem_crypto::uuid_v4();
    let address = Ipv4Addr::new(10, 128, 0, 2);
    assert!(NetworkMembership::new(Uuid::nil(), "vm", address, MembershipState::Declared, 0).is_err());
    assert!(NetworkMembership::new(id, "", address, MembershipState::Declared, 0).is_err());
    assert!(NetworkMembership::new(id, "vm", Ipv4Addr::LOCALHOST, MembershipState::Declared, 0).is_err());
    assert!(NetworkMembership::new(id, "vm", Ipv4Addr::UNSPECIFIED, MembershipState::Declared, 0).is_err());
    assert!(NetworkMembership::new(id, "vm", address, MembershipState::Declared, -1).is_err());
    assert!(NetworkMembership::new(id, "vm", address, MembershipState::Declared, 0).is_ok());
}
