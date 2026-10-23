use super::*;
use capsem_core::net::upstream_grant::UpstreamGrantClient;
use capsem_core::security_engine::approval::client::{frame_limit, ApprovalCapability};
use capsem_foundation::unix::router_channel::{DescriptorReceiver, DescriptorSender};
use capsem_proto::approval::{ApprovalClientMessage, ApprovalServerMessage};
use capsem_proto::upstream_grant::*;
use std::os::fd::AsRawFd;

#[tokio::test]
async fn real_ipc_disconnect_cancels_its_original_pending_mcp_request() {
    let temp = tempfile::tempdir().unwrap();
    let (mut dispatcher, _control) = Dispatcher::new(temp.path());
    let (broker, worker) = std::os::unix::net::UnixStream::pair().unwrap();
    let grants = Arc::new(UpstreamGrantClient::start(worker).unwrap());
    let capability = Arc::new(ApprovalCapability::new(grants));
    let receiver =
        DescriptorReceiver::<UPSTREAM_GRANT_FRAME_SIZE, UPSTREAM_GRANT_MAX_FDS>::new(broker.try_clone().unwrap())
            .unwrap();
    let sender = DescriptorSender::<UPSTREAM_GRANT_FRAME_SIZE, UPSTREAM_GRANT_MAX_FDS>::new(broker).unwrap();
    let request = receiver.recv().await.unwrap();
    let UpstreamGrantRequest::OpenApproval { request_id } = decode_upstream_grant_request(&request.bytes).unwrap()
    else {
        panic!("approval grant")
    };
    let (worker, service) = std::os::unix::net::UnixStream::pair().unwrap();
    sender
        .send(
            &encode_upstream_grant_response(&UpstreamGrantResponse::ApprovalGranted { request_id }).unwrap(),
            &[worker.as_raw_fd()],
        )
        .await
        .unwrap();
    let (_approval_sender, approval_receiver) = capsem_foundation::ipc_channel::channel_from_std_with_limit::<
        ApprovalServerMessage,
        ApprovalClientMessage,
    >(service, frame_limit())
    .unwrap();
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
        .with_approvals(Some(capability)),
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
        tokio::time::timeout(Duration::from_secs(2), approval_receiver.recv())
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
        matches!(tokio::time::timeout(Duration::from_secs(2), approval_receiver.recv()).await.expect("disconnect cancels the original approval").unwrap(), ApprovalClientMessage::Cancel { request_id: id } if id == request_id)
    );
}
