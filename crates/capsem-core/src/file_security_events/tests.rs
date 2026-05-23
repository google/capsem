use std::time::{Duration, SystemTime};

use super::*;
use capsem_logger::FileAction;

#[test]
fn builds_observe_only_file_security_event() {
    let event = FileEvent {
        timestamp: SystemTime::UNIX_EPOCH,
        action: FileAction::Created,
        path: "/root/project/src/main.rs".into(),
        size: Some(42),
        trace_id: Some("trace_file".into()),
    };

    let resolved = build_file_resolved_security_event(&event);

    assert_eq!(resolved.event.common.event_type, "file.activity");
    assert_eq!(resolved.event.common.source_engine, SourceEngine::File);
    assert_eq!(
        resolved.event.common.trace_id.as_deref(),
        Some("trace_file")
    );
    assert_eq!(resolved.event.common.event_id, "file-f667432c86acbe38");
    assert!(matches!(resolved.final_action, SecurityAction::Continue));
    assert!(resolved.steps.is_empty());
    match resolved.event.subject {
        capsem_security_engine::SecurityEventSubject::File(subject) => {
            assert_eq!(subject.operation, "created");
            assert_eq!(subject.path.as_deref(), Some("/root/project/src/main.rs"));
            assert_eq!(subject.path_class, "workspace");
            assert_eq!(subject.byte_count, Some(42));
        }
        other => panic!("expected file subject, got {other:?}"),
    }
}

#[test]
fn same_millisecond_file_events_keep_distinct_security_ids() {
    let first = FileEvent {
        timestamp: SystemTime::UNIX_EPOCH + Duration::from_millis(42),
        action: FileAction::Modified,
        path: "/root/project/src/main.rs".into(),
        size: Some(42),
        trace_id: Some("trace_file".into()),
    };
    let second = FileEvent {
        timestamp: SystemTime::UNIX_EPOCH + Duration::from_millis(42) + Duration::from_nanos(1),
        ..first.clone()
    };

    let first_resolved = build_file_resolved_security_event(&first);
    let second_resolved = build_file_resolved_security_event(&second);

    assert_ne!(
        first_resolved.event.common.event_id,
        second_resolved.event.common.event_id
    );
}

#[test]
fn classifies_restored_checkpoint_path_by_target() {
    let event = FileEvent {
        timestamp: SystemTime::UNIX_EPOCH,
        action: FileAction::Restored,
        path: "/tmp/report.md (from checkpoint-7)".into(),
        size: Some(12),
        trace_id: None,
    };

    let resolved = build_file_resolved_security_event(&event);

    match resolved.event.subject {
        capsem_security_engine::SecurityEventSubject::File(subject) => {
            assert_eq!(subject.operation, "restored");
            assert_eq!(subject.path_class, "temporary");
            assert_eq!(subject.byte_count, Some(12));
        }
        other => panic!("expected file subject, got {other:?}"),
    }
}
