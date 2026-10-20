use super::*;

fn host_event(kind: crate::events::HostEventKind, session: &str, at: i64) -> WriteOp {
    WriteOp::HostEvent(crate::events::HostEvent {
        timestamp_unix_ms: at,
        kind,
        session_id: Some(session.to_string()),
        actor: "cli".into(),
        detail: vec![0x80],
        trace_id: None,
    })
}

/// The host ledger answers "what happened to this session" only if a record
/// cannot be edited, removed or reordered without it showing.
#[tokio::test]
async fn host_events_chain_across_reopen_and_tampering_breaks_it() {
    use crate::events::HostEventKind::{SessionCreated, SessionForked, SessionStarted, SessionStopped};
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("host.db");
    {
        let writer = DbWriter::open(&db_path, 64).unwrap();
        writer.write(host_event(SessionCreated, "s1", 1)).await;
        writer.write(host_event(SessionStarted, "s1", 2)).await;
        drop(writer);
    }
    {
        // A reopened writer continues the chain from what reached disk.
        let writer = DbWriter::open(&db_path, 64).unwrap();
        writer.write(host_event(SessionForked, "s1", 3)).await;
        writer.write(host_event(SessionStopped, "s1", 4)).await;
        drop(writer);
    }
    let reader = crate::reader::DbReader::open(&db_path).unwrap();
    assert_eq!(reader.verify_host_chain().unwrap(), 4);
    let kinds: Vec<_> = reader
        .host_events()
        .unwrap()
        .into_iter()
        .map(|(event, _, _)| event.kind)
        .collect();
    assert_eq!(kinds, [SessionCreated, SessionStarted, SessionForked, SessionStopped]);
    drop(reader);

    for tamper in [
        "UPDATE host_events SET actor = 'someone-else' WHERE id = 2",
        "DELETE FROM host_events WHERE id = 2",
        "UPDATE host_events SET timestamp_unix_ms = 99 WHERE id = 3",
    ] {
        let copy = dir.path().join("tampered.db");
        std::fs::copy(&db_path, &copy).unwrap();
        rusqlite::Connection::open(&copy)
            .unwrap()
            .execute_batch(tamper)
            .unwrap();
        let reader = crate::reader::DbReader::open(&copy).unwrap();
        let error = reader.verify_host_chain().expect_err(tamper).to_string();
        assert!(error.contains("host event chain is broken"), "{tamper}: {error}");
        drop(reader);
        std::fs::remove_file(&copy).unwrap();
    }
}

#[tokio::test]
async fn the_db_object_replays_host_events_with_their_detail() {
    use crate::events::HostEventKind::{SessionCreated, SessionStopped};
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("host.db");
    let handle = crate::DbHandle::open(&db_path).unwrap();
    handle.write(host_event(SessionCreated, "s1", 1)).await.unwrap();
    let mut stopped = host_event(SessionStopped, "s1", 2);
    if let WriteOp::HostEvent(event) = &mut stopped {
        event.detail = vec![0x81, 0xa1, b'a', 0x05];
        event.trace_id = Some("t".into());
    }
    handle.write(stopped).await.unwrap();
    handle.flush().await.unwrap();

    let events = handle.host_events().await.unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[1].kind, SessionStopped);
    assert_eq!(events[1].detail, vec![0x81, 0xa1, b'a', 0x05]);
    assert_eq!(events[1].trace_id.as_deref(), Some("t"));
    assert_eq!(events[0].session_id.as_deref(), Some("s1"));
}
