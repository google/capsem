use std::time::SystemTime;

use super::*;

#[test]
fn builds_observe_only_process_exec_security_event() {
    let event = ExecEvent {
        timestamp: SystemTime::UNIX_EPOCH,
        exec_id: 42,
        command: "bash -lc 'echo hello'".into(),
        source: "api".into(),
        mcp_call_id: Some(7),
        trace_id: Some("trace_exec".into()),
        process_name: Some("capsem-agent".into()),
    };

    let resolved = build_exec_resolved_security_event(&event);

    assert_eq!(resolved.event.common.event_type, "process.exec");
    assert_eq!(resolved.event.common.source_engine, SourceEngine::Process);
    assert_eq!(resolved.event.common.activity_id.as_deref(), Some("api"));
    assert_eq!(resolved.event.common.exec_id.as_deref(), Some("42"));
    assert_eq!(resolved.event.common.mcp_call_id.as_deref(), Some("7"));
    assert!(resolved.event.common.process_id.is_none());
    assert_eq!(resolved.event.common.event_id, "process-9f67a25bbe8d30df");
    assert!(matches!(resolved.final_action, SecurityAction::Continue));
    assert!(resolved.steps.is_empty());
    match resolved.event.subject {
        capsem_security_engine::SecurityEventSubject::Process(subject) => {
            assert_eq!(subject.operation, "exec");
            assert_eq!(subject.command_class.as_deref(), Some("shell"));
        }
        other => panic!("expected process subject, got {other:?}"),
    }
}

#[test]
fn command_classifier_uses_executable_basename() {
    let event = ExecEvent {
        timestamp: SystemTime::UNIX_EPOCH,
        exec_id: 9,
        command: "/usr/bin/curl https://example.com".into(),
        source: "api".into(),
        mcp_call_id: None,
        trace_id: None,
        process_name: None,
    };

    let resolved = build_exec_resolved_security_event(&event);

    match resolved.event.subject {
        capsem_security_engine::SecurityEventSubject::Process(subject) => {
            assert_eq!(subject.command_class.as_deref(), Some("network"));
        }
        other => panic!("expected process subject, got {other:?}"),
    }
}
