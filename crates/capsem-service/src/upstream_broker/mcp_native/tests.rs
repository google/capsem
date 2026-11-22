use super::*;
use capsem_core::net::policy_config::{ActivePolicyFile, SecurityRuleProfile, SettingsFile};
use capsem_core::net::upstream_grant::UpstreamGrantClient;
use capsem_foundation::unix::fd;
use capsem_proto::mcp_aggregator::{write_frame, AggregatorClient};
use capsem_proto::mcp_contracts::{
    builtin_ledger::{self, BuiltinLedgerRecord, HttpDecision, BUILTIN_LEDGER_META_KEY},
    McpServerDef,
};
use std::collections::HashMap;
use std::time::Duration;

fn policy(allowed: bool) -> Arc<BrokerPolicy> {
    let mut active =
        ActivePolicyFile::from_settings_and_corp(&SettingsFile::default(), &SettingsFile::default()).unwrap();
    if allowed {
        active.user_rules = SecurityRuleProfile::parse_toml(
            r#"
            [profiles.rules.local_fixture]
            name = "local_fixture"
            action = "allow"
            match = 'http.host == "127.0.0.1"'
        "#,
        )
        .unwrap();
    }
    BrokerPolicy::new(
        if allowed { "allowed" } else { "blocked" }.into(),
        Arc::new(active.compile_runtime().unwrap()),
    )
}

fn definitions() -> Vec<capsem_proto::mcp_contracts::McpServerDef> {
    let builtin = crate::service_runtime::find_sibling_binary("capsem-mcp-builtin");
    let mut definitions = capsem_core::mcp::build_server_list(&Default::default(), Some(&builtin), HashMap::new());
    for definition in &mut definitions {
        definition.pool_size = Some(1);
    }
    definitions
}

fn trace_id(directory: &Path, log: &str) -> String {
    std::fs::read_to_string(directory.join(log))
        .unwrap()
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find_map(|line| line["span"]["trace_id"].as_str().map(str::to_owned))
        .expect("native log must retain its trace identity")
}

struct Session {
    client: AggregatorClient,
    _upstream: UpstreamGrantClient,
    publisher: crate::upstream_broker::PolicyPublisher,
    authority: WorkerAuthority,
    task: tokio::task::JoinHandle<crate::upstream_broker::BrokerCompletion>,
}

impl Session {
    async fn open(directory: &Path, sandbox: Option<UnixStream>, definitions: Vec<McpServerDef>) -> Self {
        std::fs::create_dir_all(directory).unwrap();
        let (pending, publisher) =
            crate::upstream_broker::PendingBroker::pair_for_session(policy(true), directory.into()).unwrap();
        let upstream = UpstreamGrantClient::start(pending.worker_stream().unwrap()).unwrap();
        let authority = WorkerAuthority::default();
        let task = pending.start(authority.grant());
        let stream = match sandbox {
            Some(endpoint) => upstream
                .open_mcp_aggregator_with_sandbox(endpoint.into())
                .await
                .unwrap(),
            None => upstream.open_mcp_aggregator().await.unwrap(),
        };
        assert!(
            upstream.open_mcp_aggregator().await.is_err(),
            "a generation may own only one aggregator"
        );
        fd::set_nonblocking(stream.as_fd(), true).unwrap();
        let (reader, mut writer) = tokio::net::UnixStream::from_std(stream).unwrap().into_split();
        write_frame(&mut writer, &definitions).await.unwrap();
        let (client, requests) = AggregatorClient::channel(capsem_proto::MCP_WORKER_QUEUE_CAPACITY);
        capsem_core::mcp::aggregator_driver::spawn(requests, writer, reader);
        Self {
            client,
            _upstream: upstream,
            publisher,
            authority,
            task,
        }
    }

    async fn start(directory: &Path) -> Self {
        let session = Self::open(directory, None, definitions()).await;
        let tools = tokio::time::timeout(Duration::from_secs(5), session.client.list_tools())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            tools.len(),
            4,
            "native builtin catalog missing; aggregator={}, builtin={}",
            std::fs::read_to_string(directory.join("mcp-aggregator.stderr.log")).unwrap_or_default(),
            std::fs::read_to_string(directory.join("mcp-builtin.stderr.log")).unwrap_or_default()
        );
        let trace = trace_id(directory, "mcp-aggregator.stderr.log");
        assert_ne!(trace, "unknown");
        assert_eq!(trace, trace_id(directory, "mcp-builtin.stderr.log"));
        session
    }

    async fn stop(self) {
        self.authority.revoke();
        let completion = tokio::time::timeout(Duration::from_secs(5), self.task)
            .await
            .unwrap()
            .unwrap();
        assert!(
            completion.cleanup_confirmed,
            "native cleanup must finish before generation completion"
        );
        assert!(
            tokio::time::timeout(Duration::from_secs(2), self.client.list_tools())
                .await
                .unwrap()
                .is_err(),
            "retired native channel stayed usable"
        );
    }
}

#[tokio::test]
async fn native_aggregator_uses_only_slots_granted_by_its_sandbox_owner() {
    use capsem_foundation::unix::router_channel::{DescriptorReceiver, DescriptorSender};
    use capsem_proto::mcp_worker::{
        decode_spawn_record, encode_spawn_record, McpSpawnRecord, MCP_SPAWN_FRAME_SIZE, MCP_SPAWN_MAX_FDS,
    };
    use serde_json::{json, Value};
    use std::os::fd::AsRawFd;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let directory = tempfile::tempdir().unwrap();
    let (owner, broker) = UnixStream::pair().unwrap();
    let owner_sender =
        DescriptorSender::<MCP_SPAWN_FRAME_SIZE, MCP_SPAWN_MAX_FDS>::new(owner.try_clone().unwrap()).unwrap();
    let owner_receiver = DescriptorReceiver::<MCP_SPAWN_FRAME_SIZE, MCP_SPAWN_MAX_FDS>::new(owner).unwrap();
    let (peer, workload) = UnixStream::pair().unwrap();
    let simulated_owner = tokio::spawn(async move {
        for (request_id, slot) in [(1, 5), (2, 3)] {
            let request = owner_receiver.recv().await.unwrap();
            assert!(request.fds.is_empty());
            assert_eq!(
                decode_spawn_record(&request.bytes).unwrap(),
                McpSpawnRecord::Sandbox { request_id, slot }
            );
            if slot == 3 {
                owner_sender
                    .send(
                        &encode_spawn_record(McpSpawnRecord::Granted { request_id }).unwrap(),
                        &[peer.as_raw_fd()],
                    )
                    .await
                    .unwrap();
                let receipt = owner_receiver.recv().await.unwrap();
                assert!(receipt.fds.is_empty());
                assert_eq!(
                    decode_spawn_record(&receipt.bytes).unwrap(),
                    McpSpawnRecord::Adopted { request_id }
                );
            } else {
                owner_sender
                    .send(
                        &encode_spawn_record(McpSpawnRecord::Denied { request_id }).unwrap(),
                        &[],
                    )
                    .await
                    .unwrap();
            }
        }
        drop(peer);
        assert!(
            owner_receiver.recv().await.is_err(),
            "sandbox owner stayed connected after retirement"
        );
    });
    let simulated_workload = tokio::spawn(async move {
        workload.set_nonblocking(true).unwrap();
        let (read, mut write) = tokio::net::UnixStream::from_std(workload).unwrap().into_split();
        let mut messages = BufReader::new(read).lines();
        let mut calls = 0;
        while let Some(line) = messages.next_line().await.unwrap() {
            let request: Value = serde_json::from_str(&line).unwrap();
            let Some(id) = request.get("id") else { continue };
            let result = match request["method"].as_str().unwrap() {
                "initialize" => json!({
                    "protocolVersion": request["params"]["protocolVersion"],
                    "capabilities": {"tools": {}}, "serverInfo": {"name": "fixture", "version": "1"},
                }),
                "tools/list" => json!({"tools":[{"name":"echo", "inputSchema":{"type":"object"}}]}),
                "resources/list" => json!({"resources":[]}),
                "prompts/list" => json!({"prompts":[]}),
                "tools/call" => {
                    assert_eq!(request["params"]["name"], "echo");
                    calls += 1;
                    json!({"content":[{"type":"text", "text":request["params"]["arguments"]["text"]}]})
                }
                other => panic!("unexpected workload method {other}"),
            };
            let mut response = serde_json::to_vec(&json!({"jsonrpc":"2.0", "id":id, "result":result})).unwrap();
            response.push(b'\n');
            write.write_all(&response).await.unwrap();
        }
        assert_eq!(calls, 1);
    });

    let mut servers = Vec::new();
    for (name, slot) in [("invalid", 4), ("unavailable", 5), ("approved", 3)] {
        servers.push(McpServerDef {
            name: name.into(),
            sandbox_slot: Some(slot),
            source: "sandbox".into(),
            enabled: true,
            url: String::new(),
            command: (name == "invalid").then(|| "/fixture-must-not-be-a-host-command".into()),
            args: vec![],
            env: Default::default(),
            headers: Default::default(),
            auth: None,
            pool_size: None,
            pool_safe_tools: vec![],
        });
    }
    servers.extend(definitions());
    let isolated = Session::open(&directory.path().join("isolated"), None, servers.clone()).await;
    let isolated_tools = tokio::time::timeout(Duration::from_secs(5), isolated.client.list_tools())
        .await
        .unwrap()
        .unwrap();
    assert!(isolated_tools.iter().any(|tool| tool.namespaced_name == "local__echo"));
    assert!(isolated_tools.iter().all(|tool| tool.server_name == "local"));

    let session = Session::open(&directory.path().join("approved"), Some(broker), servers).await;
    let tools = tokio::time::timeout(Duration::from_secs(5), session.client.list_tools())
        .await
        .unwrap()
        .unwrap();
    assert!(tools.iter().any(|tool| tool.namespaced_name == "approved__echo"));
    assert!(tools.iter().any(|tool| tool.namespaced_name == "local__echo"));
    assert!(tools
        .iter()
        .all(|tool| !matches!(tool.server_name.as_str(), "invalid" | "unavailable")));
    let response = session
        .client
        .call_tool(
            "approved__echo",
            json!({"text":"selected slot"}),
            Some(Duration::from_secs(2)),
        )
        .await
        .unwrap();
    assert_eq!(response["content"], json!([{"type":"text", "text":"selected slot"}]));
    session.stop().await;
    isolated.stop().await;
    tokio::time::timeout(Duration::from_secs(2), simulated_owner)
        .await
        .unwrap()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), simulated_workload)
        .await
        .unwrap()
        .unwrap();
}

async fn fetch(session: &Session, url: &str) -> serde_json::Value {
    tokio::time::timeout(
        Duration::from_secs(3),
        session.client.call_tool(
            "local__fetch_http",
            serde_json::json!({"url": url, "format": "raw"}),
            Some(Duration::from_secs(2)),
        ),
    )
    .await
    .unwrap()
    .unwrap()
}

#[tokio::test]
async fn service_owned_native_sessions_refresh_obey_current_policy_and_retire_independently() {
    let directory = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let url = format!("http://{}/fixture", listener.local_addr().unwrap());
    let router = axum::Router::new().route(
        "/fixture",
        axum::routing::get(|| async { "native service HTTP fixture" }),
    );
    let upstream = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let first = Session::start(&directory.path().join("first")).await;
    let second = Session::start(&directory.path().join("second")).await;
    let response = fetch(&first, &url).await;
    assert!(response["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("native service HTTP fixture"));
    let records = builtin_ledger::decode(response["_meta"][BUILTIN_LEDGER_META_KEY].clone()).unwrap();
    let [BuiltinLedgerRecord::HttpRequest(record)] = records.as_slice() else {
        panic!("missing native HTTP ledger evidence")
    };
    assert_eq!(record.decision, HttpDecision::Allowed);
    assert_eq!(record.method, "GET");
    assert_eq!(record.status_code, Some(200));
    first.client.refresh(definitions()).await.unwrap();
    let echo = first
        .client
        .call_tool(
            "local__echo",
            serde_json::json!({"text": "refreshed"}),
            Some(Duration::from_secs(2)),
        )
        .await
        .unwrap();
    assert_eq!(echo["content"][0]["text"], "refreshed");
    first.publisher.publish(policy(false)).await.unwrap();
    let response = fetch(&first, &url).await;
    assert_eq!(response["isError"], true);
    let records = builtin_ledger::decode(response["_meta"][BUILTIN_LEDGER_META_KEY].clone()).unwrap();
    let [BuiltinLedgerRecord::HttpRequest(record)] = records.as_slice() else {
        panic!("missing denial evidence")
    };
    assert_eq!(record.decision, HttpDecision::Denied);
    assert_eq!(record.method, "GET");
    first.stop().await;
    assert_ne!(
        fetch(&second, &url).await["isError"],
        true,
        "other session lost authority"
    );
    second.stop().await;
    upstream.abort();
}

#[tokio::test]
async fn native_startup_failure_still_returns_positive_owned_child_cleanup() {
    let directory = tempfile::tempdir().unwrap();
    let mut prepared = Prepared::prepare(directory.path(), Arc::new(tokio::sync::Semaphore::new(1))).unwrap();
    // A real child that cannot speak readiness must still be reaped.
    prepared.aggregator = PathBuf::from("/usr/bin/true");
    let (publisher, policy) = watch::channel(policy(true));
    let authority = WorkerAuthority::default();
    let (channel, supervisor) = prepared.open(policy, authority.grant(), None).await;
    assert!(channel.is_err());
    let supervisor = supervisor.expect("spawned native child must retain its supervisor");
    let pid = capsem_foundation::unix::process::ProcessId::try_from(supervisor.child.id().unwrap()).unwrap();
    supervisor.run(true).await.unwrap();
    let this_process = capsem_foundation::unix::process::ProcessId::try_from(std::process::id()).unwrap();
    let not_child = capsem_foundation::unix::process::child_has_exited(this_process)
        .unwrap_err()
        .raw_os_error();
    assert_eq!(
        capsem_foundation::unix::process::child_has_exited(pid)
            .unwrap_err()
            .raw_os_error(),
        not_child
    );
    drop(publisher);
}
