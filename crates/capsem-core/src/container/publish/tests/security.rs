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
            "[profiles.rules.expose]\nname = \"expose\"\naction = \"{action}\"\nmatch = 'network.mode == \"expose\"'\n\
             [profiles.rules.private]\nname = \"private\"\naction = \"{action}\"\nmatch = 'network.mode == \"private\"'"
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
            owner
                .clone()
                .accept_publication(
                    listener,
                    uuid::Uuid::new_v4(),
                    6379,
                    capsem_proto::PublicationTarget::Container,
                    stop.clone(),
                )
                .unwrap(),
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
    // Without a security context there is no feeder at all: the listener is
    // never served, so no broker exists to request a guest destination.
    let feeder = owner.clone().accept_publication(
        listener,
        uuid::Uuid::new_v4(),
        6379,
        capsem_proto::PublicationTarget::Container,
        stop.clone(),
    );
    assert!(feeder.is_err(), "a publisher without security fed a broker");
    let (feed, incoming) = mpsc::channel(1);
    drop(feed);
    let broker = tokio::spawn(broker::serve(owner.clone(), incoming, control, router, stop.clone()));
    let requested = tokio::time::timeout(Duration::from_millis(100), requests.recv()).await;
    stop.cancel();
    let result = broker.await.unwrap();
    assert!(result.is_err(), "the broker refused to run without a security context");
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
        owner
            .clone()
            .accept_publication(
                listener,
                uuid::Uuid::new_v4(),
                6379,
                capsem_proto::PublicationTarget::Container,
                stop.clone(),
            )
            .unwrap(),
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
        owner
            .clone()
            .accept_publication(
                listener,
                uuid::Uuid::new_v4(),
                6379,
                capsem_proto::PublicationTarget::Container,
                stop.clone(),
            )
            .unwrap(),
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

#[tokio::test]
async fn the_link_audit_is_this_vms_own_portless_private_flow() {
    let owner = security::authorized_publisher(capsem_config::router::RouterConfig::default());
    let audit = owner
        .private_link_audit(
            crate::security_engine::network::NetworkIdentity::parse(
                "6ba7b810-9dad-11d1-80b4-00c04fd430c8",
                "team".into(),
            )
            .unwrap(),
            Ipv4Addr::new(10, 128, 0, 3),
        )
        .unwrap();
    assert_eq!(
        audit.authorize().await.unwrap(),
        crate::security_engine::SecurityEnforcementAction::Allow
    );
    let facts = audit.facts();
    assert_eq!(facts.protocol, crate::security_engine::network::NetworkProtocol::Link);
    assert_eq!(facts.source.address, "10.128.0.3:0".parse().unwrap());
    assert_eq!(facts.destination.address, "10.128.0.3:0".parse().unwrap());
    assert_eq!(facts.source.vm.as_ref().map(|vm| vm.id.as_str()), Some("vm-id"));
    audit
        .record(
            crate::security_engine::RuntimeSecurityEventType::NetworkClose,
            crate::security_engine::network::NetworkReason::Complete,
            3,
            4,
        )
        .await
        .unwrap();
}

/// A ledger-backed engine with `rules`, for exposure lifecycle tests.
fn lifecycle_engine(dir: &tempfile::TempDir, rules: &str) -> (Arc<NetworkSecurity>, std::path::PathBuf) {
    let path = dir.path().join("session.db");
    let rules = SecurityRuleSet::compile_profile(
        &SecurityRuleProfile::parse_toml(rules).unwrap(),
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

async fn lifecycle_rows(engine: &NetworkSecurity, path: &std::path::Path) -> String {
    engine.db.flush_checked().await.unwrap();
    capsem_logger::DbReader::open(path)
        .unwrap()
        .query_raw_with_params(
            "SELECT network_id, connection_id, event_json FROM transport_events WHERE event_type = 'network.lifecycle'",
            &[],
        )
        .unwrap()
}

fn free_port() -> u16 {
    std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

const BLOCK_REDIS: &str = "[profiles.rules.no_redis]\nname = \"no_redis\"\naction = \"block\"\n\
                           match = 'network.action != \"revoked\" && network.destination.port == \"6379\"'";
const ASK_REDIS: &str = "[profiles.rules.ask_redis]\nname = \"ask_redis\"\naction = \"ask\"\n\
                         match = 'network.action != \"revoked\" && network.destination.port == \"6379\"'";

const BLOCK_CONTAINER_IMAGE: &str = "[profiles.rules.no_registry_image]\nname = \"no_registry_image\"\n\
                                     action = \"block\"\n\
                                     match = 'container.registry == \"registry.example\" && \
                                     container.image == \"registry.example/private/app:1\"'";

#[tokio::test]
async fn a_refused_container_pull_is_audited_with_image_identity() {
    let dir = tempfile::tempdir().unwrap();
    let (engine, path) = lifecycle_engine(&dir, BLOCK_CONTAINER_IMAGE);
    let owner = Publisher::default().with_security("vm-id".into(), "builder".into(), engine.clone());

    let error = owner
        .admit_container_pull("registry.example/private/app:1".into(), "registry.example".into(), None)
        .await
        .expect_err("the image rule must refuse the pull");

    assert!(
        error.is::<crate::container::publish::ContainerPullRefused>(),
        "{error:#}"
    );
    let rows = lifecycle_rows(&engine, &path).await;
    assert!(rows.contains("container_pull"), "{rows}");
    assert!(rows.contains("registry.example/private/app:1"), "{rows}");
    assert!(rows.contains("registry.example"), "{rows}");
    let decisions = capsem_logger::DbReader::open(&path)
        .unwrap()
        .query_raw_with_params(
            "SELECT event_json FROM security_decision_events WHERE event_type = 'network.lifecycle'",
            &[],
        )
        .unwrap();
    assert!(decisions.contains("registry.example/private/app:1"), "{decisions}");
    assert!(decisions.contains("registry.example"), "{decisions}");
}

#[tokio::test]
async fn a_container_pull_whose_audit_cannot_be_admitted_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let (engine, _path) = lifecycle_engine(&dir, "");
    engine.db.shutdown_blocking();
    let owner = Publisher::default().with_security("vm-id".into(), "builder".into(), engine);

    let error = owner
        .admit_container_pull("registry.example/app:1".into(), "registry.example".into(), None)
        .await
        .expect_err("a closed audit ledger must refuse the pull");

    assert!(
        !error.is::<crate::container::publish::ContainerPullRefused>(),
        "audit failure must stay distinct from a policy refusal: {error:#}"
    );
}

#[tokio::test]
async fn a_refused_exposure_is_audited_and_leaves_its_port_unbound() {
    for (rules, refusal) in [(BLOCK_REDIS, "blocked by policy"), (ASK_REDIS, "needs approval")] {
        let dir = tempfile::tempdir().unwrap();
        let (engine, path) = lifecycle_engine(&dir, rules);
        let owner = Arc::new(Publisher::default().with_security("vm-id".into(), "redis".into(), engine.clone()));
        let (control, _requests) = mpsc::channel(8);
        let port = free_port();
        let error = owner
            .publish(port, 6379, capsem_proto::PublicationTarget::Container, control)
            .await
            .err()
            .expect("the rules refuse this exposure");
        assert!(error.is::<crate::container::publish::ExposureRefused>(), "{error:#}");
        assert!(error.to_string().contains(refusal), "{error:#}");
        std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, port)).expect("a refused exposure keeps no listener");
        assert!(owner.publications().is_empty());
        let rows = lifecycle_rows(&engine, &path).await;
        assert!(rows.contains("published") && rows.contains("6379"), "{rows}");
        owner.shutdown().await;
    }
}

#[tokio::test]
async fn an_exposure_whose_audit_cannot_be_admitted_is_not_opened() {
    let dir = tempfile::tempdir().unwrap();
    let (engine, _path) = lifecycle_engine(&dir, "");
    engine.db.shutdown_blocking();
    let owner = Arc::new(Publisher::default().with_security("vm-id".into(), "redis".into(), engine));
    let (control, _requests) = mpsc::channel(8);
    let port = free_port();
    let error = owner
        .publish(port, 6379, capsem_proto::PublicationTarget::Container, control)
        .await
        .err()
        .expect("a closed ledger refuses the exposure");
    assert!(
        !error.is::<crate::container::publish::ExposureRefused>(),
        "an audit failure is not a policy refusal: {error:#}"
    );
    std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, port)).expect("no listener survives a failed audit");
    owner.shutdown().await;
}

#[tokio::test]
async fn a_revoked_exposure_closes_then_is_audited_under_its_publication() {
    let dir = tempfile::tempdir().unwrap();
    let (engine, path) = lifecycle_engine(&dir, BLOCK_REDIS);
    let owner = Arc::new(Publisher::default().with_security("vm-id".into(), "redis".into(), engine.clone()));
    let cancellation = CancellationToken::new();
    let publication_id = uuid::Uuid::new_v4();
    let task = tokio::spawn(std::future::pending::<()>());
    owner.declared.insert(registry::Declared {
        host_port: 41234,
        guest_port: 6379,
        target: capsem_proto::PublicationTarget::Container,
        handle: Publication {
            host_port: 41234,
            router_pid: 0,
            publication_id,
            task: task.abort_handle(),
            cancellation: cancellation.clone(),
        },
    });
    assert!(
        owner.revoke(41234).await.unwrap(),
        "revoke proceeds even under a blocking rule"
    );
    assert!(cancellation.is_cancelled(), "the publication was closed");
    let rows = lifecycle_rows(&engine, &path).await;
    assert!(
        rows.contains("revoked") && rows.contains(&publication_id.to_string()),
        "{rows}"
    );
    task.abort();
    owner.shutdown().await;
}

#[tokio::test]
async fn a_saved_exposure_the_rules_now_refuse_is_forgotten_on_restore() {
    let dir = tempfile::tempdir().unwrap();
    let (engine, path) = lifecycle_engine(&dir, BLOCK_REDIS);
    let port = free_port();
    std::fs::write(
        dir.path().join("published-ports.json"),
        format!(r#"[{{"host":{port},"guest":6379,"target":"container"}}]"#),
    )
    .unwrap();
    let owner = Arc::new(
        Publisher::for_session(dir.path(), capsem_config::router::RouterConfig::default())
            .unwrap()
            .with_security("vm-id".into(), "redis".into(), engine.clone()),
    );
    let (control, _requests) = mpsc::channel(8);
    assert_eq!(owner.restore(control).await.unwrap(), 0);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("published-ports.json")).unwrap(),
        "[]"
    );
    let rows = lifecycle_rows(&engine, &path).await;
    assert!(rows.contains("restored"), "{rows}");
    std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, port)).expect("nothing listens for a refused restore");
    owner.shutdown().await;
}
