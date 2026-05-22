use std::time::SystemTime;

use capsem_security_engine::{
    CelEnforcementEvaluator, CelEnforcementRule, SecurityDecisionAction, SecurityEngine,
};

use super::*;

#[test]
fn builds_inline_blockable_process_exec_security_event() {
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
    assert_eq!(
        resolved.event.common.enforceability,
        Enforceability::InlineBlockable
    );
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
fn process_exec_security_evaluation_allows_when_no_engine_is_installed() {
    let event = ExecEvent {
        timestamp: SystemTime::UNIX_EPOCH,
        exec_id: 43,
        command: "python3 -c 'print(1)'".into(),
        source: "api".into(),
        mcp_call_id: None,
        trace_id: Some("trace_exec_no_engine".into()),
        process_name: Some("capsem-agent".into()),
    };

    let evaluation = evaluate_exec_security_event(&event, None);

    assert!(evaluation.allow_guest_exec);
    assert!(evaluation.denial_message.is_none());
    assert!(matches!(
        evaluation.resolved_event.final_action,
        SecurityAction::Continue
    ));
}

#[test]
fn process_exec_security_evaluation_blocks_matching_cel_rule() {
    let event = ExecEvent {
        timestamp: SystemTime::UNIX_EPOCH,
        exec_id: 44,
        command: "bash -lc 'echo blocked'".into(),
        source: "api".into(),
        mcp_call_id: None,
        trace_id: Some("trace_exec_blocked".into()),
        process_name: Some("capsem-agent".into()),
    };
    let mut engine = SecurityEngine::default();
    engine.set_enforcement(Box::new(
        CelEnforcementEvaluator::compile(vec![CelEnforcementRule {
            id: "process.block-shell".into(),
            pack_id: Some("corp-enforcement".into()),
            condition:
                "process.activity.operation == 'exec' && process.activity.command_class == 'shell'"
                    .into(),
            decision: SecurityDecisionAction::Block,
            reason: Some("shell commands are blocked".into()),
        }])
        .unwrap(),
    ));
    let engine = std::sync::Mutex::new(engine);

    let evaluation = evaluate_exec_security_event(&event, Some(&engine));

    assert!(!evaluation.allow_guest_exec);
    assert_eq!(
        evaluation.denial_message.as_deref(),
        Some("process exec blocked by process.block-shell: shell commands are blocked")
    );
    assert!(matches!(
        evaluation.resolved_event.final_action,
        SecurityAction::Block(_)
    ));
    assert_eq!(evaluation.resolved_event.steps.len(), 1);
    assert_eq!(
        evaluation.resolved_event.steps[0].rule_id.as_deref(),
        Some("process.block-shell")
    );
}

#[test]
fn process_exec_security_evaluation_default_denies_ask_without_confirm_resolver() {
    let event = ExecEvent {
        timestamp: SystemTime::UNIX_EPOCH,
        exec_id: 45,
        command: "bash -lc 'echo ask'".into(),
        source: "api".into(),
        mcp_call_id: None,
        trace_id: Some("trace_exec_ask".into()),
        process_name: Some("capsem-agent".into()),
    };
    let mut engine = SecurityEngine::default();
    engine.set_enforcement(Box::new(
        CelEnforcementEvaluator::compile(vec![CelEnforcementRule {
            id: "process.ask-shell".into(),
            pack_id: Some("corp-enforcement".into()),
            condition:
                "process.activity.operation == 'exec' && process.activity.command_class == 'shell'"
                    .into(),
            decision: SecurityDecisionAction::Ask,
            reason: Some("shell commands require approval".into()),
        }])
        .unwrap(),
    ));
    let engine = std::sync::Mutex::new(engine);

    let evaluation = evaluate_exec_security_event(&event, Some(&engine));

    assert!(!evaluation.allow_guest_exec);
    assert_eq!(
        evaluation.denial_message.as_deref(),
        Some(
            "process exec blocked by process.ask-shell: shell commands require approval; default denied because no confirm resolver is configured"
        )
    );
    assert!(matches!(
        evaluation.resolved_event.final_action,
        SecurityAction::Block(_)
    ));
    assert!(evaluation
        .resolved_event
        .steps
        .iter()
        .any(|step| step.kind == ResolvedEventStepKind::Confirm
            && step.status == StepStatus::Applied
            && step.rule_id.as_deref() == Some("process.ask-shell")));
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
