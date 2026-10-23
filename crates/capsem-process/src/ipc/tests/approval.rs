use super::*;
use capsem_proto::approval::ApprovalClientMessage;

#[tokio::test]
async fn real_ipc_disconnect_cancels_its_original_pending_mcp_request() {
    let temp = tempfile::tempdir().unwrap();
    let (mut dispatcher, _control) = Dispatcher::new(temp.path());
    let peer = crate::tests::ApprovalPeer::new().await;
    let runtime = Arc::get_mut(&mut dispatcher.mcp_runtime).unwrap();
    let profile = capsem_core::net::policy_config::SecurityRuleProfile::parse_toml(
        r#"
        [profiles.rules.ask_mcp]
        name = "ask_mcp"
        action = "ask"
        match = 'mcp.method == "tools/call" && mcp.response.valid == "false"'
    "#,
    )
    .unwrap();
    let rules = capsem_core::net::policy_config::SecurityRuleSet::compile_profile(
        &profile,
        capsem_core::net::policy_config::SecurityRuleSource::User,
    )
    .unwrap();
    *runtime.security_rules.write().unwrap() = Arc::new(rules);
    runtime.endpoint = Arc::new(
        capsem_core::net::mitm_proxy::McpEndpointState::new(
            runtime.aggregator.clone(),
            Arc::clone(&runtime.security_rules),
            Arc::clone(&runtime.plugin_policy),
            Arc::new(tokio::sync::Semaphore::new(4)),
            capsem_core::net::mitm_proxy::McpTimeouts::from_env(),
        )
        .with_approvals(Some(Arc::clone(&peer.capability))),
    );
    let (process, service) = tokio::net::UnixStream::pair().unwrap();
    let owner = dispatcher.connection(process);
    let mut service = service.into_std().unwrap();
    let (tx, rx) = tokio::task::spawn_blocking(move || {
        capsem_foundation::ipc_handshake::negotiate_initiator(&mut service, "capsem-service-test", "").unwrap();
        channel_from_std::<ServiceToProcess, ProcessToService>(service).unwrap()
    })
    .await
    .unwrap();
    tx.send(ServiceToProcess::McpCallTool {
        id: 1,
        namespaced_name: "fixture__echo".into(),
        arguments_json: "{}".into(),
    })
    .await
    .unwrap();
    let ApprovalClientMessage::Begin { request_id, .. } =
        tokio::time::timeout(Duration::from_secs(2), peer.receiver.recv())
            .await
            .unwrap()
            .unwrap()
    else {
        panic!("begin")
    };
    drop(tx);
    drop(rx);
    tokio::time::timeout(Duration::from_secs(2), owner)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        matches!(tokio::time::timeout(Duration::from_secs(2), peer.receiver.recv()).await.expect("disconnect cancels the original approval").unwrap(), ApprovalClientMessage::Cancel { request_id: id } if id == request_id)
    );
}
