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

#[tokio::test]
async fn host_access_denial_preserves_actor_target_and_detail_across_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let event = crate::HostEvent {
        timestamp_unix_ms: 42,
        kind: crate::HostEventKind::AccessDenied,
        session_id: Some("target-session".into()),
        actor: "agent:access-id".into(),
        detail: vec![0x81, 0xa1, b'a', 0x05],
        trace_id: Some("request-trace".into()),
    };
    {
        let db = crate::DbHandle::open(&path).unwrap();
        db.write(WriteOp::HostEvent(event.clone())).await.unwrap();
        db.flush().await.unwrap();
        assert_eq!(db.host_events().await.unwrap().as_slice(), std::slice::from_ref(&event));
    }
    let reader = crate::reader::DbReader::open(&path).unwrap();
    assert_eq!(reader.host_events().unwrap(), [event]);
}

#[tokio::test]
async fn host_event_pages_bound_filter_and_resume_by_insertion_order() {
    use crate::HostEventKind::{GrantMinted, GrantRevoked};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.db");
    let db = crate::DbHandle::open(&path).unwrap();
    for (index, kind) in [GrantMinted, GrantRevoked, GrantMinted, GrantMinted]
        .into_iter()
        .enumerate()
    {
        // Deliberately reverse timestamps: insertion order owns pagination.
        db.write(host_event(kind, "target", 100 - index as i64)).await.unwrap();
    }
    db.flush().await.unwrap();
    drop(db);
    let db = crate::DbHandle::open(&path).unwrap();
    let first = db.host_event_page(GrantMinted, 0, 2).await.unwrap();
    assert_eq!(first.iter().map(|(id, _)| *id).collect::<Vec<_>>(), [1, 3]);
    assert_eq!(first[0].1.actor, "cli");
    assert_eq!(first[0].1.detail, [0x80]);
    assert_eq!(first[1].1.timestamp_unix_ms, 98);
    let second = db.host_event_page(GrantMinted, 3, 2).await.unwrap();
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].0, 4);
    assert!(db.host_event_page(GrantMinted, 4, 2).await.unwrap().is_empty());
    assert_eq!(db.host_event_page(GrantRevoked, 0, 2).await.unwrap()[0].0, 2);
    assert!(db.host_event_page(GrantMinted, -1, 2).await.is_err());
    assert!(db.host_event_page(GrantMinted, 0, 0).await.is_err());
    assert!(db.host_event_page(GrantMinted, 0, usize::MAX).await.is_err());
}
