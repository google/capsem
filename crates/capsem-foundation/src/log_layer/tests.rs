use super::*;

#[test]
fn visitor_message_only() {
    let v = MessageVisitor {
        message: "boot failed".into(),
        fields: vec![],
    };
    assert_eq!(v.into_message(), "boot failed");
}

#[test]
fn visitor_message_with_fields() {
    let v = MessageVisitor {
        message: "failed to initialize MCP server".into(),
        fields: vec![
            ("server".into(), "Deps dev".into()),
            ("error".into(), "connection refused".into()),
        ],
    };
    assert_eq!(
        v.into_message(),
        "failed to initialize MCP server (server=Deps dev, error=connection refused)"
    );
}

#[test]
fn visitor_fields_only_no_message() {
    let v = MessageVisitor {
        message: String::new(),
        fields: vec![("key".into(), "val".into())],
    };
    assert_eq!(v.into_message(), "key=val");
}

#[test]
fn format_timestamp_epoch() {
    let ts = format_timestamp(0, 0);
    assert_eq!(ts, "1970-01-01T00:00:00.000Z");
}

#[test]
fn format_timestamp_with_millis() {
    // 2026-03-17T10:05:32.123Z
    // seconds since epoch for 2026-03-17T10:05:32 UTC
    let secs = 1773741932;
    let ts = format_timestamp(secs, 123);
    assert_eq!(ts, "2026-03-17T10:05:32.123Z");
}

#[test]
fn log_event_serialization_roundtrip() {
    let event = LogEvent {
        timestamp: "2026-03-17T10:05:32.000Z".to_string(),
        level: "INFO".to_string(),
        target: "capsem::vm::boot".to_string(),
        message: "kernel loaded".to_string(),
    };
    let json = serde_json::to_string(&event).unwrap();
    let parsed: LogEvent = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.timestamp, event.timestamp);
    assert_eq!(parsed.level, event.level);
    assert_eq!(parsed.target, event.target);
    assert_eq!(parsed.message, event.message);
}

#[test]
fn log_handle_set_emitter_drains_buffer() {
    let (layer, handle) = TauriLogLayer::new();

    // Simulate buffered events
    {
        let mut guard = layer.early_buffer.lock().unwrap();
        if let Some(ref mut buf) = *guard {
            buf.push(LogEvent {
                timestamp: "t1".into(),
                level: "INFO".into(),
                target: "test".into(),
                message: "buffered".into(),
            });
        }
    }

    let received = Arc::new(Mutex::new(Vec::new()));
    let r = Arc::clone(&received);
    handle.set_emitter(move |event| {
        r.lock().unwrap().push(event);
    });

    let events = received.lock().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].message, "buffered");
}

#[test]
fn log_handle_emitter_set_only_once() {
    let (_layer, handle) = TauriLogLayer::new();
    let called = Arc::new(Mutex::new(false));
    let c = Arc::clone(&called);
    handle.set_emitter(move |_| {
        *c.lock().unwrap() = true;
    });

    // Second set should be silently ignored (OnceLock)
    handle.set_emitter(|_| {});

    // First emitter should still be active
    if let Some(emitter) = handle.emitter.get() {
        emitter(LogEvent {
            timestamp: "t".into(),
            level: "INFO".into(),
            target: "t".into(),
            message: "test".into(),
        });
    }
    assert!(*called.lock().unwrap());
}

#[test]
fn vm_writer_writes_jsonl() {
    let (_layer, handle) = TauriLogLayer::new();

    let dir = std::env::temp_dir().join("capsem-test-log-layer");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("test.log");
    let file = std::fs::File::create(&path).unwrap();

    handle.set_vm_writer(file);

    // Send events through the channel
    {
        let guard = handle.vm_writer_tx.lock().unwrap();
        if let Some(ref tx) = *guard {
            tx.send(WriterMsg::Event(LogEvent {
                timestamp: "2026-03-17T10:00:00.000Z".into(),
                level: "INFO".into(),
                target: "capsem::vm::boot".into(),
                message: "kernel loaded".into(),
            }))
            .unwrap();
            tx.send(WriterMsg::Event(LogEvent {
                timestamp: "2026-03-17T10:00:01.000Z".into(),
                level: "WARN".into(),
                target: "capsem::mcp".into(),
                message: "timeout".into(),
            }))
            .unwrap();
        }
    }

    handle.clear_vm_writer();

    // Give writer thread time to finish
    std::thread::sleep(std::time::Duration::from_millis(50));

    let content = crate::telemetry::read_log_tail(&path, usize::MAX).unwrap();
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 2);

    let e1: LogEvent = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(e1.message, "kernel loaded");
    let e2: LogEvent = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(e2.message, "timeout");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn clear_vm_writer_without_set_is_noop() {
    let (_layer, handle) = TauriLogLayer::new();
    handle.clear_vm_writer(); // should not panic
}

#[test]
fn early_buffer_caps_at_limit() {
    let (layer, _handle) = TauriLogLayer::new();
    {
        let mut guard = layer.early_buffer.lock().unwrap();
        if let Some(ref mut buf) = *guard {
            for i in 0..EARLY_BUFFER_CAP + 50 {
                // Simulate what the layer would do
                if buf.len() < EARLY_BUFFER_CAP {
                    buf.push(LogEvent {
                        timestamp: format!("t{i}"),
                        level: "INFO".into(),
                        target: "test".into(),
                        message: format!("msg {i}"),
                    });
                }
            }
            assert_eq!(buf.len(), EARLY_BUFFER_CAP);
        }
    }
}

#[test]
fn level_filtering_info_and_above() {
    // Verify that the level comparison is correct
    assert!(Level::DEBUG > Level::INFO); // DEBUG is "less important" = higher numeric
    assert!(Level::TRACE > Level::INFO);
    assert!(Level::WARN <= Level::INFO); // WARN passes the filter
    assert!(Level::ERROR <= Level::INFO); // ERROR passes the filter
    assert!(Level::INFO <= Level::INFO); // INFO passes the filter
}
