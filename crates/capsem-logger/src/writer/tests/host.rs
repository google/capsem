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

/// Database order and complete events survive a writer restart.
#[tokio::test]
async fn host_events_remain_ordered_across_reopen() {
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
        // A reopened writer continues the durable insertion order.
        let writer = DbWriter::open(&db_path, 64).unwrap();
        writer.write(host_event(SessionForked, "s1", 3)).await;
        writer.write(host_event(SessionStopped, "s1", 4)).await;
        drop(writer);
    }
    let reader = crate::reader::DbReader::open(&db_path).unwrap();
    let kinds: Vec<_> = reader
        .host_events()
        .unwrap()
        .into_iter()
        .map(|event| event.kind)
        .collect();
    assert_eq!(kinds, [SessionCreated, SessionStarted, SessionForked, SessionStopped]);
    drop(reader);
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
