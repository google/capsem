use super::*;
use capsem_core::net::policy_config::{ActivePolicyFile, SecurityRuleProfile, SettingsFile};
use capsem_core::net::upstream_grant::UpstreamGrantClient;
use capsem_foundation::unix::fd;
use capsem_proto::mcp_aggregator::{write_frame, AggregatorClient};
use capsem_proto::mcp_contracts::builtin_ledger::{self, BuiltinLedgerRecord, HttpDecision, BUILTIN_LEDGER_META_KEY};
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
    async fn start(directory: &Path) -> Self {
        std::fs::create_dir_all(directory).unwrap();
        let (pending, publisher) =
            crate::upstream_broker::PendingBroker::pair_for_session(policy(true), directory.into()).unwrap();
        let upstream = UpstreamGrantClient::start(pending.worker_stream().unwrap()).unwrap();
        let authority = WorkerAuthority::default();
        let task = pending.start(authority.grant());
        let stream = upstream.open_mcp_aggregator().await.unwrap();
        assert!(
            upstream.open_mcp_aggregator().await.is_err(),
            "a generation may own only one aggregator"
        );
        fd::set_nonblocking(stream.as_fd(), true).unwrap();
        let (reader, mut writer) = tokio::net::UnixStream::from_std(stream).unwrap().into_split();
        write_frame(&mut writer, &definitions()).await.unwrap();
        let (client, requests) = AggregatorClient::channel(capsem_proto::MCP_WORKER_QUEUE_CAPACITY);
        capsem_core::mcp::aggregator_driver::spawn(requests, writer, reader);
        let tools = tokio::time::timeout(Duration::from_secs(5), client.list_tools())
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
        Self {
            client,
            _upstream: upstream,
            publisher,
            authority,
            task,
        }
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
    let (channel, supervisor) = prepared.open(policy, authority.grant()).await;
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
