use std::time::{Duration, SystemTime};

use capsem_logger::{FileAction, FileEvent};
use capsem_security_engine::{SecurityAction, SecurityEventSubject, SourceEngine};

use super::*;

#[test]
fn builds_observe_only_file_security_event() {
    let event = FileEvent {
        timestamp: SystemTime::UNIX_EPOCH,
        action: FileAction::Created,
        path: "/root/project/src/main.rs".into(),
        size: Some(42),
        trace_id: Some("trace_file".into()),
    };
    let identity = FileEngineIdentity {
        vm_id: Some("vm_1".into()),
        session_id: Some("session_1".into()),
        profile_id: Some("coding".into()),
        profile_revision: Some("2026.05.23".into()),
        user_id: Some("user_1".into()),
    };

    let resolved = build_file_resolved_security_event(&event, &identity);

    assert_eq!(resolved.event.common.event_type, "file.activity");
    assert_eq!(resolved.event.common.source_engine, SourceEngine::File);
    assert_eq!(
        resolved.event.common.trace_id.as_deref(),
        Some("trace_file")
    );
    assert_eq!(resolved.event.common.event_id, "file-f667432c86acbe38");
    assert_eq!(resolved.event.common.vm_id.as_deref(), Some("vm_1"));
    assert_eq!(resolved.event.common.profile_id.as_deref(), Some("coding"));
    assert!(matches!(resolved.final_action, SecurityAction::Continue));
    assert!(resolved.steps.is_empty());
    match resolved.event.subject {
        SecurityEventSubject::File(subject) => {
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
    let identity = FileEngineIdentity::default();

    let first_resolved = build_file_resolved_security_event(&first, &identity);
    let second_resolved = build_file_resolved_security_event(&second, &identity);

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

    let resolved = build_file_resolved_security_event(&event, &FileEngineIdentity::default());

    match resolved.event.subject {
        SecurityEventSubject::File(subject) => {
            assert_eq!(subject.operation, "restored");
            assert_eq!(subject.path_class, "temporary");
            assert_eq!(subject.byte_count, Some(12));
        }
        other => panic!("expected file subject, got {other:?}"),
    }
}

#[test]
fn classifies_common_path_families() {
    assert_eq!(file_path_class("/workspace/app/main.py"), "workspace");
    assert_eq!(file_path_class("/tmp/output.txt"), "temporary");
    assert_eq!(file_path_class("/etc/passwd"), "system");
    assert_eq!(file_path_class("/opt/tool/config"), "absolute");
    assert_eq!(file_path_class("relative.txt"), "relative");
}
