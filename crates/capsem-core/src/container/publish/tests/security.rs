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
        generation: owner.generation,
        id: old.id,
    };
    assert!(owner.pending_connection(flow));
    owner.control_lost();
    owner.control_ready().unwrap();
    assert!(!owner.pending_connection(flow));
    let (fresh, _receiver) = owner.request(&source, close).unwrap();
    assert!(owner.pending_connection(capsem_proto::router::FlowKey {
        generation: owner.generation,
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
