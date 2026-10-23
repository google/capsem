use super::*;
use crate::net::proxy_engine::{LocalProxyCredentials, ProxyEngine, ProxyPolicyHandle};
use crate::security_engine::approval::client::{frame_limit, ApprovalCapability, ApprovalClient};
use capsem_foundation::ipc_channel;
use capsem_proto::approval::{ApprovalClientMessage, ApprovalOutcome, ApprovalServerMessage};

#[tokio::test]
async fn approved_http_pending_identity_reaches_completed_network_ledger() {
    let dir = tempfile::tempdir().unwrap();
    let db = Arc::new(DbWriter::open(&dir.path().join("session.db"), 64).unwrap());
    let profile = SecurityRuleProfile::parse_toml(
        r#"
        [profiles.rules.ask_http]
        name = "ask_http"
        action = "ask"
        match = 'http.host == "api.anthropic.com"'
    "#,
    )
    .unwrap();
    let rules = SecurityRuleSet::compile_profile(&profile, SecurityRuleSource::User).unwrap();
    let snapshot = policy_snapshot(rules);
    let (worker, coordinator) = std::os::unix::net::UnixStream::pair().unwrap();
    let capability = Arc::new(ApprovalCapability::connected(
        ApprovalClient::from_stream(worker).unwrap(),
    ));
    let (send, receive) = ipc_channel::channel_from_std_with_limit::<ApprovalServerMessage, ApprovalClientMessage>(
        coordinator,
        frame_limit(),
    )
    .unwrap();
    let engine = ProxyEngine::new(
        ProxyPolicyHandle::new(Arc::try_unwrap(snapshot).ok().unwrap()),
        Arc::clone(&db) as Arc<dyn crate::net::proxy_engine::ProxyLedger>,
        Arc::new(LocalProxyCredentials),
    )
    .with_approvals(Some(capability));
    let snapshot = engine.policy().snapshot();
    let event = SecurityEvent::new(RuntimeSecurityEventType::HttpRequest).with_http(HttpSecurityEvent {
        host: Some("api.anthropic.com".into()),
        method: Some("POST".into()),
        ..Default::default()
    });
    let call = tokio::spawn(async move { engine.evaluate(&engine.policy().snapshot(), event).await.unwrap() });
    let ApprovalClientMessage::Begin { request_id, request } =
        tokio::time::timeout(std::time::Duration::from_secs(2), receive.recv())
            .await
            .unwrap()
            .unwrap()
    else {
        panic!("begin")
    };
    send.send(ApprovalServerMessage {
        request_id,
        outcome: ApprovalOutcome::Approved,
    })
    .await
    .unwrap();
    let evaluation = call.await.unwrap();
    assert!(evaluation.enforcement.is_allowed());
    let decision = super::super::super::SecurityBoundaryDecisionFields::default().with_evaluation(&evaluation);
    let mut context = anthropic_req_ctx();
    context.policy_snapshot = snapshot;
    context.model_traffic = false;
    context.approval_event_id = decision.approval_event_id;
    context.policy_mode = decision.policy_mode;
    context.policy_action = decision.policy_action;
    context.policy_rule = decision.policy_rule;
    context.policy_reason = decision.policy_reason;
    let hook = TelemetryHook::new(Arc::new(TelemetryDeps {
        db: Arc::clone(&db),
        credentials: Arc::new(LocalProxyCredentials),
        pricing: Arc::new(PricingTable::load()),
        trace_state: Arc::new(Mutex::new(TraceState::new())),
    }));
    let mut state = HookState::default();
    let conn = any_conn();
    *ctx_for(&mut state, &conn).state::<Option<TelemetryRequestContext>>(|| None) = Some(context);
    complete_response(&hook, &mut state, &conn).await;
    db.flush_checked().await.unwrap();
    let conn = rusqlite::Connection::open(dir.path().join("session.db")).unwrap();
    let linked: i64 = conn.query_row(
        "SELECT COUNT(*) FROM security_ask_events AS ask JOIN net_events AS net ON net.event_id=ask.event_id WHERE ask.ask_id=?1",
        [&request.ask_id], |row| row.get(0),
    ).unwrap();
    assert_eq!(
        linked, 1,
        "original approved HTTP request lost its causal identity before telemetry"
    );
}
