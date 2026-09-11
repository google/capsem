use super::*;

use crate::net::policy_config::{
    DetectionLevel, SecurityPluginConfig, SecurityPluginMode, SecurityRuleProfile, SecurityRuleSet, SecurityRuleSource,
};
use crate::security_engine::network::ledger::NetworkSecurity;
use std::sync::RwLock;

fn engine(action: &str, plugin_block: bool) -> Arc<NetworkSecurity> {
    let text = if action.is_empty() {
        String::new()
    } else {
        format!(
            "[profiles.rules.expose]\nname = \"expose\"\naction = \"{action}\"\nmatch = 'network.mode == \"expose\"'"
        )
    };
    let rules = SecurityRuleSet::compile_profile(
        &SecurityRuleProfile::parse_toml(&text).unwrap(),
        SecurityRuleSource::User,
    )
    .unwrap();
    let mut plugins = std::collections::BTreeMap::new();
    if plugin_block {
        plugins.insert(
            "dummy_post_allow".into(),
            SecurityPluginConfig {
                mode: SecurityPluginMode::Block,
                detection_level: DetectionLevel::High,
            },
        );
    }
    Arc::new(NetworkSecurity {
        db: Arc::new(capsem_logger::DbWriter::open_in_memory(128).unwrap()),
        rules: Arc::new(RwLock::new(Arc::new(rules))),
        plugins: Arc::new(RwLock::new(Arc::new(plugins))),
    })
}

pub(super) fn authorized_publisher(budgets: capsem_config::router::RouterConfig) -> Publisher {
    let owner =
        Publisher::configured(budgets)
            .unwrap()
            .with_security("vm-id".into(), "redis".into(), engine("allow", false));
    owner.control_ready().unwrap();
    owner
}

#[tokio::test]
async fn deny_ask_plugin_and_closed_audit_never_open_a_guest_destination() {
    for (action, plugin_block, closed) in [
        ("", false, false),
        ("block", false, false),
        ("ask", false, false),
        ("allow", true, false),
        ("allow", false, true),
    ] {
        let security = engine(action, plugin_block);
        if closed {
            security.db.shutdown_blocking();
        }
        let owner = Arc::new(Publisher::default().with_security("vm-id".into(), "redis".into(), security));
        owner.control_ready().unwrap();
        let (parent, _child) = StdUnixStream::pair().unwrap();
        let router = Arc::new(companion::Router::new(
            0,
            capsem_foundation::unix::router_channel::Sender::new(parent).unwrap(),
            CancellationToken::new(),
        ));
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let _client = tokio::net::TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (control, mut requests) = mpsc::channel(32);
        let stop = CancellationToken::new();
        let broker = tokio::spawn(broker::serve(
            owner.clone(),
            6379,
            listener,
            control,
            router,
            stop.clone(),
        ));
        // Denial may enqueue cleanup, but must never request a guest socket.
        let opened = tokio::time::timeout(Duration::from_millis(100), async {
            while let Some(request) = requests.recv().await {
                if matches!(request, ServiceToProcess::ConnectPort { .. }) {
                    return true;
                }
            }
            false
        })
        .await
        .unwrap_or(false);
        stop.cancel();
        let result = broker.await.unwrap();
        owner.shutdown().await;
        assert!(
            !opened,
            "destination opened: action={action}, plugin={plugin_block}, closed={closed}"
        );
        if !closed {
            result.unwrap();
        }
        assert!(owner.pending.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn a_new_control_lease_cannot_admit_old_queued_setup() {
    let owner = Arc::new(authorized_publisher(capsem_config::router::RouterConfig::default()));
    let source = source_fixture();
    let (close, _reports) = mpsc::channel(8);
    let (old, _receiver) = owner.request(&source, close.clone()).unwrap();
    let flow = capsem_proto::router::FlowKey {
        generation: owner.generation.get(),
        id: old.id,
    };
    assert!(owner.pending_connection(flow));
    owner.control_lost();
    owner.control_ready().unwrap();
    assert!(!owner.pending_connection(flow));
    let (fresh, _receiver) = owner.request(&source, close).unwrap();
    assert!(owner.pending_connection(capsem_proto::router::FlowKey {
        generation: owner.generation.get(),
        id: fresh.id
    }));
    drop(old);
    drop(fresh);
    owner.shutdown().await;
}

#[tokio::test]
async fn missing_security_context_never_requests_a_guest_connection() {
    let owner = Arc::new(Publisher::default());
    let (parent, _child) = StdUnixStream::pair().unwrap();
    let router = Arc::new(companion::Router::new(
        0,
        capsem_foundation::unix::router_channel::Sender::new(parent).unwrap(),
        CancellationToken::new(),
    ));
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let address = listener.local_addr().unwrap();
    let (control, mut requests) = mpsc::channel(32);
    let stop = CancellationToken::new();
    let _client = tokio::net::TcpStream::connect(address).await.unwrap();
    let broker = tokio::spawn(broker::serve(
        owner.clone(),
        6379,
        listener,
        control,
        router,
        stop.clone(),
    ));
    let requested = tokio::time::timeout(Duration::from_millis(100), requests.recv()).await;
    stop.cancel();
    let _result = broker.await.unwrap();
    owner.shutdown().await;
    assert!(
        requested.is_err() || requested.unwrap().is_none(),
        "missing security context opened a guest destination"
    );
    assert!(owner.pending.lock().unwrap().is_empty());
}

/// An allow-everything engine writing to a real session ledger, so a test can
/// read back what the broker recorded about a connection.
fn file_engine(dir: &tempfile::TempDir) -> (Arc<NetworkSecurity>, std::path::PathBuf) {
    let path = dir.path().join("session.db");
    let rules = SecurityRuleSet::compile_profile(
        &SecurityRuleProfile::parse_toml(
            "[profiles.rules.expose]\nname = \"expose\"\naction = \"allow\"\nmatch = 'network.mode == \"expose\"'",
        )
        .unwrap(),
        SecurityRuleSource::User,
    )
    .unwrap();
    let engine = Arc::new(NetworkSecurity {
        db: Arc::new(capsem_logger::DbWriter::open(&path, 128).unwrap()),
        rules: Arc::new(RwLock::new(Arc::new(rules))),
        plugins: Arc::new(RwLock::new(Arc::new(std::collections::BTreeMap::new()))),
    });
    (engine, path)
}

/// The `network.connect_result` rows the ledger holds, once one exists.
async fn recorded_connect_results(engine: &NetworkSecurity, path: &std::path::Path) -> String {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        engine.db.flush_checked().await.unwrap();
        let rows = capsem_logger::DbReader::open(path)
            .unwrap()
            .query_raw_with_params(
                "SELECT event_json FROM transport_events WHERE event_type = 'network.connect_result'",
                &[],
            )
            .unwrap();
        if rows.contains("\"reason\"") || tokio::time::Instant::now() >= deadline {
            return rows;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn fake_router() -> Arc<companion::Router> {
    let (parent, child) = StdUnixStream::pair().unwrap();
    std::mem::forget(child);
    Arc::new(companion::Router::new(
        0,
        capsem_foundation::unix::router_channel::Sender::new(parent).unwrap(),
        CancellationToken::new(),
    ))
}

/// The audit must say what happened. Before setup can begin the guest bridge
/// may simply not be there (never ready) or go away (lost while the setup
/// waits); neither is the post-setup generation recheck that
/// `stale_generation` names, and both used to be recorded as it.
#[tokio::test]
async fn a_control_lease_that_never_came_up_is_audited_as_unreachable() {
    let dir = tempfile::tempdir().unwrap();
    let (engine, path) = file_engine(&dir);
    let owner = Arc::new(Publisher::default().with_security("vm-id".into(), "redis".into(), engine.clone()));
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let _client = tokio::net::TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let (control, _requests) = mpsc::channel(32);
    let stop = CancellationToken::new();
    let broker = tokio::spawn(broker::serve(
        owner.clone(),
        6379,
        listener,
        control,
        fake_router(),
        stop.clone(),
    ));

    let rows = recorded_connect_results(&engine, &path).await;
    stop.cancel();
    broker.await.unwrap().unwrap();
    owner.shutdown().await;
    // event_json travels as an escaped string inside the query result, so
    // match the reason token rather than its JSON spelling.
    assert!(rows.contains("unreachable"), "{rows}");
    assert!(!rows.contains("stale_generation"), "{rows}");
}

#[tokio::test]
async fn a_control_lease_lost_while_setup_waits_is_audited_as_cancelled() {
    let dir = tempfile::tempdir().unwrap();
    let (engine, path) = file_engine(&dir);
    let owner = Arc::new(Publisher::default().with_security("vm-id".into(), "redis".into(), engine.clone()));
    owner.control_ready().unwrap();
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let _client = tokio::net::TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let (control, mut requests) = mpsc::channel(32);
    let stop = CancellationToken::new();
    let broker = tokio::spawn(broker::serve(
        owner.clone(),
        6379,
        listener,
        control,
        fake_router(),
        stop.clone(),
    ));
    // Setup reached the guest request and is waiting on the bridge's reply.
    let request = tokio::time::timeout(Duration::from_secs(5), requests.recv())
        .await
        .unwrap();
    assert!(matches!(request, Some(ServiceToProcess::ConnectPort { .. })));
    owner.control_lost();

    let rows = recorded_connect_results(&engine, &path).await;
    stop.cancel();
    broker.await.unwrap().unwrap();
    owner.shutdown().await;
    assert!(rows.contains("cancelled"), "{rows}");
    assert!(!rows.contains("stale_generation"), "{rows}");
}
