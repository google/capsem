use super::super::events::{Event, EventKind, EventMask};
use super::super::hooks::{Hook, HookCtx, HookOutcome, StopAction};
use super::*;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

// ── Test hooks ──────────────────────────────────────────────────────

/// Records its name + the kind it saw, in the order it ran.
struct Recorder {
    name: &'static str,
    interest: EventMask,
    priority: i32,
    log: Arc<Mutex<Vec<&'static str>>>,
}

impl Hook for Recorder {
    fn name(&self) -> &'static str {
        self.name
    }
    fn interest(&self) -> EventMask {
        self.interest
    }
    fn priority(&self) -> i32 {
        self.priority
    }
    fn on_event<'a, 'b>(
        &'a self,
        _ev: &'b mut Event<'_>,
        _ctx: &'b mut HookCtx<'_>,
    ) -> Pin<Box<dyn std::future::Future<Output = HookOutcome> + Send + 'b>>
    where
        'a: 'b,
    {
        let name = self.name;
        let log = self.log.clone();
        Box::pin(async move {
            log.lock().unwrap().push(name);
            HookOutcome::Continue
        })
    }
}

/// Increments a counter every time it fires; useful for "did you see
/// the emitted event?" assertions on L2/L3 dispatch.
struct Counter {
    name: &'static str,
    interest: EventMask,
    count: Arc<AtomicUsize>,
}

impl Hook for Counter {
    fn name(&self) -> &'static str {
        self.name
    }
    fn interest(&self) -> EventMask {
        self.interest
    }
    fn on_event<'a, 'b>(
        &'a self,
        _ev: &'b mut Event<'_>,
        _ctx: &'b mut HookCtx<'_>,
    ) -> Pin<Box<dyn std::future::Future<Output = HookOutcome> + Send + 'b>>
    where
        'a: 'b,
    {
        let count = self.count.clone();
        Box::pin(async move {
            count.fetch_add(1, Ordering::SeqCst);
            HookOutcome::Continue
        })
    }
}

/// Stops the pipeline with `StopAction::Drop`.
struct Stopper;

impl Hook for Stopper {
    fn name(&self) -> &'static str {
        "stopper"
    }
    fn interest(&self) -> EventMask {
        EventMask::single(EventKind::RawRequestEnd)
    }
    fn on_event<'a, 'b>(
        &'a self,
        _ev: &'b mut Event<'_>,
        _ctx: &'b mut HookCtx<'_>,
    ) -> Pin<Box<dyn std::future::Future<Output = HookOutcome> + Send + 'b>>
    where
        'a: 'b,
    {
        Box::pin(async move { HookOutcome::Stop(StopAction::Drop) })
    }
}

/// L1 hook that emits an L2 SseEvent and remembers whether emit succeeded.
struct Emitter {
    saw_emit_ok: Arc<AtomicUsize>,
}

impl Hook for Emitter {
    fn name(&self) -> &'static str {
        "emitter"
    }
    fn interest(&self) -> EventMask {
        EventMask::single(EventKind::RawResponseChunk)
    }
    fn on_event<'a, 'b>(
        &'a self,
        _ev: &'b mut Event<'_>,
        ctx: &'b mut HookCtx<'_>,
    ) -> Pin<Box<dyn std::future::Future<Output = HookOutcome> + Send + 'b>>
    where
        'a: 'b,
    {
        let saw = self.saw_emit_ok.clone();
        Box::pin(async move {
            let mut sse = crate::net::parsers::sse_parser::SseEvent {
                event_type: Some("test".into()),
                data: "hello".into(),
            };
            if ctx.emit(Event::SseEvent(&mut sse)).await.is_ok() {
                saw.fetch_add(1, Ordering::SeqCst);
            }
            HookOutcome::Continue
        })
    }
}

/// L3 hook that tries to emit at L1 (must fail with CycleAttempt).
struct CycleViolator {
    cycle_rejected: Arc<AtomicUsize>,
}

impl Hook for CycleViolator {
    fn name(&self) -> &'static str {
        "cycle_violator"
    }
    fn interest(&self) -> EventMask {
        EventMask::single(EventKind::AiCallEnd)
    }
    fn on_event<'a, 'b>(
        &'a self,
        _ev: &'b mut Event<'_>,
        ctx: &'b mut HookCtx<'_>,
    ) -> Pin<Box<dyn std::future::Future<Output = HookOutcome> + Send + 'b>>
    where
        'a: 'b,
    {
        let rejected = self.cycle_rejected.clone();
        Box::pin(async move {
            let mut chunk = bytes::Bytes::from_static(b"oops");
            let res = ctx.emit(Event::RawResponseChunk(&mut chunk)).await;
            if matches!(
                res,
                Err(super::super::hooks::EmitError::CycleAttempt { .. })
            ) {
                rejected.fetch_add(1, Ordering::SeqCst);
            }
            HookOutcome::Continue
        })
    }
}

/// Hook that uses ctx.state to count calls within a connection.
struct CarryOver;

#[derive(Default)]
struct CarryState {
    calls: usize,
    last_chunk: Vec<u8>,
}

impl Hook for CarryOver {
    fn name(&self) -> &'static str {
        "carry_over"
    }
    fn interest(&self) -> EventMask {
        EventMask::single(EventKind::RawRequestChunk)
    }
    fn on_event<'a, 'b>(
        &'a self,
        ev: &'b mut Event<'_>,
        ctx: &'b mut HookCtx<'_>,
    ) -> Pin<Box<dyn std::future::Future<Output = HookOutcome> + Send + 'b>>
    where
        'a: 'b,
    {
        // Synchronously mutate; future is trivial.
        let new_chunk = if let Event::RawRequestChunk(c) = ev {
            Some(c.to_vec())
        } else {
            None
        };
        let st = ctx.state::<CarryState>(CarryState::default);
        st.calls += 1;
        if let Some(c) = new_chunk {
            st.last_chunk = c;
        }
        Box::pin(async move { HookOutcome::Continue })
    }
}

// ── Tests ───────────────────────────────────────────────────────────

#[tokio::test]
async fn dispatch_runs_only_matching_hooks() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let pipeline = Pipeline::builder()
        .register(Arc::new(Recorder {
            name: "raw",
            interest: EventMask::single(EventKind::RawRequestHead),
            priority: 0,
            log: log.clone(),
        }))
        .register(Arc::new(Recorder {
            name: "sse",
            interest: EventMask::single(EventKind::SseEvent),
            priority: 0,
            log: log.clone(),
        }))
        .build();

    let mut chunk = bytes::Bytes::from_static(b"x");
    dispatch_one(&pipeline, Event::RawResponseChunk(&mut chunk), None).await;
    assert!(
        log.lock().unwrap().is_empty(),
        "no hook should match RawResponseChunk"
    );

    let req = http::Request::new(()).into_parts().0;
    let mut req = req;
    dispatch_one(&pipeline, Event::RawRequestHead(&mut req), None).await;
    assert_eq!(*log.lock().unwrap(), vec!["raw"]);
}

#[tokio::test]
async fn dispatch_orders_by_priority_then_registration() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let pipeline = Pipeline::builder()
        // Registered first, but high priority -> runs last.
        .register(Arc::new(Recorder {
            name: "low",
            interest: EventMask::single(EventKind::RawRequestEnd),
            priority: 100,
            log: log.clone(),
        }))
        // Negative priority -> runs first.
        .register(Arc::new(Recorder {
            name: "high",
            interest: EventMask::single(EventKind::RawRequestEnd),
            priority: -10,
            log: log.clone(),
        }))
        // Same priority as the first registration -> stable, runs in
        // registration order between them.
        .register(Arc::new(Recorder {
            name: "tie",
            interest: EventMask::single(EventKind::RawRequestEnd),
            priority: 100,
            log: log.clone(),
        }))
        .build();

    dispatch_one(&pipeline, Event::RawRequestEnd, None).await;
    assert_eq!(*log.lock().unwrap(), vec!["high", "low", "tie"]);
}

#[tokio::test]
async fn stop_short_circuits_subsequent_hooks() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let pipeline = Pipeline::builder()
        .register(Arc::new(Stopper))
        .register(Arc::new(Recorder {
            name: "after_stop",
            interest: EventMask::single(EventKind::RawRequestEnd),
            priority: 100,
            log: log.clone(),
        }))
        .build();

    let outcome = dispatch_one(&pipeline, Event::RawRequestEnd, None).await;
    assert!(matches!(
        outcome,
        DispatchOutcome::Stopped(StopAction::Drop)
    ));
    assert!(
        log.lock().unwrap().is_empty(),
        "after_stop must not fire when an earlier hook returns Stop"
    );
}

#[tokio::test]
async fn emit_dispatches_l2_event_to_l2_subscribers() {
    let count = Arc::new(AtomicUsize::new(0));
    let saw_emit = Arc::new(AtomicUsize::new(0));
    let pipeline = Pipeline::builder()
        .register(Arc::new(Emitter {
            saw_emit_ok: saw_emit.clone(),
        }))
        .register(Arc::new(Counter {
            name: "sse_consumer",
            interest: EventMask::single(EventKind::SseEvent),
            count: count.clone(),
        }))
        .build();

    let mut chunk = bytes::Bytes::from_static(b"data: x\n\n");
    dispatch_one(&pipeline, Event::RawResponseChunk(&mut chunk), None).await;
    assert_eq!(saw_emit.load(Ordering::SeqCst), 1);
    assert_eq!(count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn cycle_attempt_rejected_when_l3_emits_l1() {
    let rejected = Arc::new(AtomicUsize::new(0));
    let l1_count = Arc::new(AtomicUsize::new(0));
    let pipeline = Pipeline::builder()
        .register(Arc::new(CycleViolator {
            cycle_rejected: rejected.clone(),
        }))
        .register(Arc::new(Counter {
            name: "l1_consumer_should_never_fire",
            interest: EventMask::single(EventKind::RawResponseChunk),
            count: l1_count.clone(),
        }))
        .build();

    let mut model_call = Box::new(capsem_logger::ModelCall {
        event_id: None,
        timestamp: std::time::SystemTime::UNIX_EPOCH,
        provider: "anthropic".into(),
        protocol: Some("anthropic".into()),
        model: None,
        process_name: None,
        pid: None,
        method: "POST".into(),
        path: "/v1/messages".into(),
        stream: false,
        system_prompt_preview: None,
        messages_count: 0,
        tools_count: 0,
        request_bytes: 0,
        request_body_preview: None,
        request_body_full: None,
        message_id: None,
        status_code: Some(200),
        text_content: None,
        thinking_content: None,
        response_body_full: None,
        stop_reason: None,
        input_tokens: None,
        output_tokens: None,
        usage_details: Default::default(),
        duration_ms: 0,
        response_bytes: 0,
        estimated_cost_usd: 0.0,
        trace_id: None,
        credential_ref: None,
        tool_calls: Vec::new(),
        tool_responses: Vec::new(),
    });
    dispatch_one(&pipeline, Event::AiCallEnd(&mut model_call), None).await;
    assert_eq!(
        rejected.load(Ordering::SeqCst),
        1,
        "L3 -> L1 must be rejected"
    );
    assert_eq!(
        l1_count.load(Ordering::SeqCst),
        0,
        "L1 hook must not fire from rejected emit"
    );
}

#[tokio::test]
async fn ctx_state_persists_across_chunks() {
    let pipeline = Pipeline::builder().register(Arc::new(CarryOver)).build();

    // Drive two chunks through the same connection-level state.
    let mut state = super::super::hooks::HookState::default();
    let conn = super::super::hooks::ConnMeta::default();
    let mut a = bytes::Bytes::from_static(b"first chunk");
    pipeline
        .dispatch(Event::RawRequestChunk(&mut a), &mut state, None, &conn)
        .await;
    let mut b = bytes::Bytes::from_static(b"second chunk");
    pipeline
        .dispatch(Event::RawRequestChunk(&mut b), &mut state, None, &conn)
        .await;

    // After two dispatches, the hook's CarryState should report 2 calls
    // and "second chunk" as the latest payload.
    let cs = state.peek::<CarryState>().expect("state slot present");
    assert_eq!(cs.calls, 2);
    assert_eq!(&cs.last_chunk, b"second chunk");
}

// ── ChunkHook tests ────────────────────────────────────────────────

/// Sync chunk hook that uppercases response bytes in place.
struct UppercaseHook;

impl super::super::hooks::ChunkHook for UppercaseHook {
    fn name(&self) -> &'static str {
        "uppercase"
    }
    fn on_response_chunk(
        &self,
        chunk: &mut bytes::Bytes,
        _ctx: &mut super::super::hooks::ChunkCtx<'_>,
    ) {
        let upper: bytes::Bytes = chunk.iter().map(|b| b.to_ascii_uppercase()).collect();
        *chunk = upper;
    }
}

/// Sync chunk hook that counts chunks + bytes via ChunkCtx::state.
struct CountChunks;

#[derive(Default)]
struct CountState {
    chunks: usize,
    bytes: usize,
    ended: bool,
}

impl super::super::hooks::ChunkHook for CountChunks {
    fn name(&self) -> &'static str {
        "count_chunks"
    }
    fn on_response_chunk(
        &self,
        chunk: &mut bytes::Bytes,
        ctx: &mut super::super::hooks::ChunkCtx<'_>,
    ) {
        let st = ctx.state::<CountState>(CountState::default);
        st.chunks += 1;
        st.bytes += chunk.len();
    }
    fn on_response_end(&self, ctx: &mut super::super::hooks::ChunkCtx<'_>) {
        ctx.state::<CountState>(CountState::default).ended = true;
    }
}

#[test]
fn chunk_hooks_run_in_registration_order_and_can_rewrite() {
    let pipeline = Pipeline::builder()
        .register_chunk(Arc::new(UppercaseHook))
        .register_chunk(Arc::new(CountChunks))
        .build();

    assert!(pipeline.has_chunk_hooks());

    let mut state = super::super::hooks::HookState::default();
    let conn = super::super::hooks::ConnMeta::default();

    let mut a = bytes::Bytes::from_static(b"hello");
    pipeline.dispatch_response_chunk(&mut a, &mut state, &conn, None);
    assert_eq!(&a[..], b"HELLO", "uppercase ran before count saw the chunk");

    let mut b = bytes::Bytes::from_static(b"world!");
    pipeline.dispatch_response_chunk(&mut b, &mut state, &conn, None);
    assert_eq!(&b[..], b"WORLD!");

    pipeline.dispatch_response_end(&mut state, &conn, None);

    let cs = state.peek::<CountState>().expect("count state present");
    assert_eq!(cs.chunks, 2);
    // Bytes counted AFTER UppercaseHook ran -- length is unchanged
    // here but a length-changing hook would propagate to count's view.
    assert_eq!(cs.bytes, 5 + 6);
    assert!(cs.ended, "on_response_end fired after the last chunk");
}

#[test]
fn empty_pipeline_has_no_chunk_hooks() {
    let pipeline = Pipeline::builder().build();
    assert!(!pipeline.has_chunk_hooks());

    // Calling dispatch on an empty chunk-hook list is a no-op (cheap
    // body wrappers can short-circuit on `has_chunk_hooks()`).
    let mut state = super::super::hooks::HookState::default();
    let conn = super::super::hooks::ConnMeta::default();
    let mut chunk = bytes::Bytes::from_static(b"unchanged");
    pipeline.dispatch_response_chunk(&mut chunk, &mut state, &conn, None);
    assert_eq!(&chunk[..], b"unchanged");
}

#[tokio::test]
async fn dispatch_records_hook_metrics() {
    use metrics_util::debugging::{DebugValue, DebuggingRecorder, Snapshotter};

    let recorder = DebuggingRecorder::new();
    let snapshotter: Snapshotter = recorder.snapshotter();
    // Thread-local recorder guard -- doesn't pollute parallel tests.
    let _guard = ::metrics::set_default_local_recorder(&recorder);

    let pipeline = Pipeline::builder()
        .register(Arc::new(Recorder {
            name: "metrics_target",
            interest: EventMask::single(EventKind::RawRequestEnd),
            priority: 0,
            log: Arc::new(Mutex::new(Vec::new())),
        }))
        .build();
    dispatch_one(&pipeline, Event::RawRequestEnd, None).await;

    let snap = snapshotter.snapshot().into_vec();
    let inv_count: u64 = snap
        .iter()
        .find_map(|(k, _, _, v)| match (k.key().name(), v) {
            ("mitm.hook_invocations_total", DebugValue::Counter(c)) => Some(*c),
            _ => None,
        })
        .expect("mitm.hook_invocations_total counter present");
    assert_eq!(inv_count, 1, "exactly one hook invocation recorded");

    let dur_present = snap.iter().any(|(k, _, _, v)| {
        matches!(v, DebugValue::Histogram(_)) && k.key().name() == "mitm.hook_duration_ms"
    });
    assert!(dur_present, "mitm.hook_duration_ms histogram recorded");
}

#[tokio::test]
async fn trace_id_visible_to_hook() {
    struct TraceCheck {
        seen: Arc<Mutex<Option<String>>>,
    }
    impl Hook for TraceCheck {
        fn name(&self) -> &'static str {
            "trace_check"
        }
        fn interest(&self) -> EventMask {
            EventMask::single(EventKind::RawRequestEnd)
        }
        fn on_event<'a, 'b>(
            &'a self,
            _ev: &'b mut Event<'_>,
            ctx: &'b mut HookCtx<'_>,
        ) -> Pin<Box<dyn std::future::Future<Output = HookOutcome> + Send + 'b>>
        where
            'a: 'b,
        {
            *self.seen.lock().unwrap() = ctx.trace_id().map(str::to_owned);
            Box::pin(async move { HookOutcome::Continue })
        }
    }

    let seen = Arc::new(Mutex::new(None));
    let pipeline = Pipeline::builder()
        .register(Arc::new(TraceCheck { seen: seen.clone() }))
        .build();
    dispatch_one(&pipeline, Event::RawRequestEnd, Some("abc123".to_string())).await;
    assert_eq!(seen.lock().unwrap().as_deref(), Some("abc123"));
}
