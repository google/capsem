use super::super::events::EventLayer;
use super::*;

#[test]
fn emit_error_display_includes_layers() {
    let e = EmitError::CycleAttempt {
        from: EventLayer::L3,
        to: EventLayer::L1,
    };
    let msg = e.to_string();
    assert!(msg.contains("L3"));
    assert!(msg.contains("L1"));
}

#[test]
fn registration_matches_only_declared_kinds() {
    use crate::net::mitm_proxy::events::{EventKind, EventMask};

    struct Stub;
    impl Hook for Stub {
        fn name(&self) -> &'static str {
            "stub"
        }
        fn interest(&self) -> EventMask {
            EventMask::single(EventKind::SseEvent)
        }
        fn on_event<'a, 'b>(
            &'a self,
            _ev: &'b mut crate::net::mitm_proxy::events::Event<'_>,
            _ctx: &'b mut HookCtx<'_>,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = HookOutcome> + Send + 'b>>
        where
            'a: 'b,
        {
            Box::pin(async { HookOutcome::Continue })
        }
    }

    let hook: ArcHook = std::sync::Arc::new(Stub);
    let reg = Registration {
        hook: hook.clone(),
        priority: 0,
        registration_order: 0,
        interest: hook.interest(),
    };
    assert!(reg.matches(EventKind::SseEvent));
    assert!(!reg.matches(EventKind::JsonRpcMessage));
    assert!(!reg.matches(EventKind::AiCallEnd));
}
