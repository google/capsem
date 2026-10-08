use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use super::super::{http_request_security_event, HttpRequestSecurityEventInput};
use super::*;
use crate::net::policy::UpstreamOverride;
use crate::net::policy_config::{SecurityRuleAction, SecurityRuleProfile, SecurityRuleSet, SecurityRuleSource};

const DEFAULT_RULES: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../capsem-config/src/default_provider_rules.toml"
));
const LOCAL_NETWORK: &str = "profiles.rules.default_000_local_network";

struct TestGrants {
    address: SocketAddr,
    next_id: AtomicU64,
    selection_releases: Arc<AtomicUsize>,
    stream_releases: Arc<AtomicUsize>,
}

impl TcpUpstreamGrants for TestGrants {
    fn resolve(&self, _protocol: Protocol, _host: &str, _port: u16) -> TcpResolveGrantFuture<'_> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let releases = Arc::clone(&self.selection_releases);
        Box::pin(async move {
            Ok(TcpGrantSelection::new(
                id,
                Protocol::Http,
                Some("127.0.0.1".parse().unwrap()),
                move || {
                    releases.fetch_add(1, Ordering::Relaxed);
                },
            ))
        })
    }

    fn connect(&self, _selection_id: u64) -> TcpConnectGrantFuture<'_> {
        let address = self.address;
        let releases = Arc::clone(&self.stream_releases);
        Box::pin(async move {
            let stream = TcpStream::connect(address).await?;
            Ok(GrantedTcpStream::new(stream, move || {
                releases.fetch_add(1, Ordering::Relaxed);
            }))
        })
    }
}

fn default_rules() -> SecurityRuleSet {
    let profile = SecurityRuleProfile::parse_toml(DEFAULT_RULES).expect("defaults parse");
    SecurityRuleSet::compile_profile(&profile, SecurityRuleSource::BuiltinDefault).expect("defaults compile")
}

/// The first enforcement rule the built-in defaults reach for a guest
/// request to `domain:port` judged as `target`.
fn decision(target: &UpstreamTarget, domain: &str, port: u16) -> (String, SecurityRuleAction) {
    let event = http_request_security_event(HttpRequestSecurityEventInput {
        domain,
        upstream_ip: target.judged_ip(domain),
        upstream_port: port,
        method: "GET",
        path: "/",
        query: None,
        ai_provider: None,
        headers: http::HeaderMap::new(),
        body: None,
    });
    let rules = default_rules();
    let evaluation = rules.evaluate(&event).expect("event evaluates");
    let rule = evaluation.enforcement_rules()[0];
    (rule.rule_id.clone(), rule.action)
}

async fn select(resolver: &UpstreamResolver, policy: &NetworkMechanics, domain: &str, port: u16) -> UpstreamTarget {
    UpstreamTarget::select(
        resolver,
        policy,
        Protocol::Tls,
        domain,
        port,
        &UpstreamCache::default(),
        None,
    )
    .await
}

#[tokio::test]
async fn fresh_selection_is_the_same_target_the_mitm_path_judges() {
    let resolver = UpstreamResolver::system().with_fixed_answer(
        "same.example",
        vec!["93.184.216.34".parse().unwrap(), "10.1.2.3".parse().unwrap()],
    );
    let policy = NetworkMechanics::new();

    let trusted = UpstreamTarget::resolve(&resolver, &policy, "same.example", 443).await;
    let mitm = select(&resolver, &policy, "same.example", 443).await;

    assert_eq!(trusted, mitm);
    assert_eq!(trusted.judged_ip("same.example"), Some("10.1.2.3".parse().unwrap()));
    assert_eq!(trusted.protocol(Protocol::Tls), Protocol::Tls);
}

#[tokio::test]
async fn a_public_looking_name_that_resolves_locally_is_asked_by_the_default_guard() {
    for (name, address) in [
        ("loopback.example", "127.0.0.1"),
        ("gateway.example", "127.0.0.1"),
        ("metadata.example", "169.254.169.254"),
        ("intranet.example", "10.1.2.3"),
        ("v6-loopback.example", "::1"),
        ("v6-link-local.example", "fe80::1"),
        ("mapped.example", "::ffff:127.0.0.1"),
    ] {
        let resolver = UpstreamResolver::system().with_fixed_answer(name, vec![address.parse().unwrap()]);
        let target = select(&resolver, &NetworkMechanics::new(), name, 80).await;
        assert_eq!(
            decision(&target, name, 80),
            (LOCAL_NETWORK.to_string(), SecurityRuleAction::Ask),
            "{name} -> {address} must be judged as the local address it reaches"
        );
    }
}

#[tokio::test]
async fn a_name_that_resolves_publicly_is_not_a_local_network_request() {
    let resolver =
        UpstreamResolver::system().with_fixed_answer("public.example", vec!["93.184.216.34".parse().unwrap()]);
    let target = select(&resolver, &NetworkMechanics::new(), "public.example", 443).await;
    assert_eq!(
        target,
        UpstreamTarget::Resolved(vec!["93.184.216.34:443".parse().unwrap()])
    );
    assert_eq!(
        decision(&target, "public.example", 443),
        ("profiles.rules.default_http".to_string(), SecurityRuleAction::Allow)
    );
}

#[tokio::test]
async fn a_mixed_answer_is_judged_by_its_local_member() {
    let resolver = UpstreamResolver::system().with_fixed_answer(
        "rebind.example",
        vec!["93.184.216.34".parse().unwrap(), "127.0.0.1".parse().unwrap()],
    );
    let target = select(&resolver, &NetworkMechanics::new(), "rebind.example", 80).await;
    assert_eq!(target.judged_ip("rebind.example"), Some("127.0.0.1".parse().unwrap()));
    assert_eq!(decision(&target, "rebind.example", 80).1, SecurityRuleAction::Ask);
}

#[tokio::test]
async fn ip_literals_keep_their_address_and_unresolvable_names_have_none() {
    let resolver = UpstreamResolver::system();
    let literal = select(&resolver, &NetworkMechanics::new(), "10.0.0.7", 8080).await;
    assert_eq!(
        literal,
        UpstreamTarget::Resolved(vec!["10.0.0.7:8080".parse().unwrap()])
    );
    assert_eq!(literal.judged_ip("10.0.0.7"), Some("10.0.0.7".parse().unwrap()));

    let missing = select(&resolver, &NetworkMechanics::new(), "no-such-host.invalid", 80).await;
    assert!(matches!(missing, UpstreamTarget::Unresolved(_)), "{missing:?}");
    assert_eq!(missing.judged_ip("no-such-host.invalid"), None);
    let refused = missing.connect().await.expect_err("an unresolved name is never dialed");
    assert_eq!(refused.kind(), std::io::ErrorKind::NotFound);
}

/// Administrator routing is trusted: the configured target is dialed as
/// written, nothing is resolved for it, and the rules see what they saw.
#[tokio::test]
async fn an_upstream_override_is_left_alone() {
    let mut policy = NetworkMechanics::new();
    policy.upstream_overrides = BTreeMap::from([(
        "replay.example:443".to_string(),
        UpstreamOverride {
            dial: "127.0.0.1:9".to_string(),
            protocol: UpstreamOverrideProtocol::Http,
        },
    )]);
    // A fixed answer for the name proves the override wins over resolution.
    let resolver = UpstreamResolver::system().with_fixed_answer("replay.example", vec!["10.9.9.9".parse().unwrap()]);
    let target = select(&resolver, &policy, "replay.example", 443).await;
    assert_eq!(
        target,
        UpstreamTarget::Override {
            dial: "127.0.0.1:9".to_string(),
            protocol: Protocol::Http,
        }
    );
    assert_eq!(target.judged_ip("replay.example"), None);
    assert_eq!(target.protocol(Protocol::Tls), Protocol::Http);
}

#[tokio::test]
async fn connect_reaches_exactly_the_judged_address_and_pins_the_peer() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    // The name never resolves; only the judged address can be reached.
    let resolver = UpstreamResolver::system().with_fixed_answer("pinned.invalid", vec![address.ip()]);
    let target = select(&resolver, &NetworkMechanics::new(), "pinned.invalid", address.port()).await;
    let ((stream, pinned), accepted) = tokio::join!(async { target.connect().await.unwrap() }, listener.accept());
    let (_, client) = accepted.unwrap();
    assert_eq!(stream.peer_addr().unwrap(), address);
    assert_eq!(client, stream.local_addr().unwrap());
    assert_eq!(pinned, UpstreamTarget::Resolved(vec![address]));
}

async fn sender() -> SendRequest<ProxyBoxBody> {
    let (client, _server) = tokio::io::duplex(64);
    let (sender, connection) = hyper::client::conn::http1::handshake(hyper_util::rt::TokioIo::new(client))
        .await
        .unwrap();
    tokio::spawn(connection);
    sender
}

/// A keep-alive request stays on the address its connection was judged and
/// opened with; a new DNS answer cannot move it. Another host or port on the
/// same guest connection is resolved and judged on its own.
#[tokio::test]
async fn keep_alive_requests_reuse_only_their_own_pinned_upstream() {
    let pinned = UpstreamTarget::Resolved(vec!["93.184.216.34:80".parse().unwrap()]);
    let cache = UpstreamCache::new(Some(CachedUpstream::new(
        "site.example",
        80,
        pinned.clone(),
        sender().await,
    )));
    let rebound = UpstreamResolver::system()
        .with_fixed_answer("site.example", vec!["127.0.0.1".parse().unwrap()])
        .with_fixed_answer("other.example", vec!["127.0.0.1".parse().unwrap()]);
    let policy = NetworkMechanics::new();

    let same = UpstreamTarget::select(&rebound, &policy, Protocol::Http, "site.example", 80, &cache, None).await;
    assert_eq!(same, pinned, "the same host stays on the address it was judged with");

    let other_port =
        UpstreamTarget::select(&rebound, &policy, Protocol::Http, "site.example", 8080, &cache, None).await;
    let other_host = UpstreamTarget::select(&rebound, &policy, Protocol::Http, "other.example", 80, &cache, None).await;
    assert_eq!(
        other_host.judged_ip("other.example"),
        Some("127.0.0.1".parse().unwrap())
    );

    let cached = cache.lock().await.take().unwrap();
    assert!(cached.serves("site.example", 80, &same));
    assert!(!cached.serves("site.example", 8080, &other_port));
    assert!(!cached.serves("other.example", 80, &other_host));
    assert!(
        !cached.serves("other.example", 80, &pinned),
        "the host is part of the key"
    );
}

#[tokio::test]
async fn a_trusted_override_preempts_an_existing_resolved_connection() {
    let pinned = UpstreamTarget::Resolved(vec!["93.184.216.34:443".parse().unwrap()]);
    let cache = UpstreamCache::new(Some(CachedUpstream::new("site.example", 443, pinned, sender().await)));
    let mut policy = NetworkMechanics::new();
    policy.upstream_overrides = BTreeMap::from([(
        "site.example:443".to_string(),
        UpstreamOverride {
            dial: "127.0.0.1:3713".to_string(),
            protocol: UpstreamOverrideProtocol::Http,
        },
    )]);

    let selected = UpstreamTarget::select(
        &UpstreamResolver::system(),
        &policy,
        Protocol::Tls,
        "site.example",
        443,
        &cache,
        None,
    )
    .await;

    assert_eq!(
        selected,
        UpstreamTarget::Override {
            dial: "127.0.0.1:3713".to_string(),
            protocol: Protocol::Http,
        }
    );
}

#[tokio::test]
async fn brokered_selection_is_judged_connected_pinned_and_released() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let selection_releases = Arc::new(AtomicUsize::new(0));
    let stream_releases = Arc::new(AtomicUsize::new(0));
    let grants = TestGrants {
        address: listener.local_addr().unwrap(),
        next_id: AtomicU64::new(1),
        selection_releases: Arc::clone(&selection_releases),
        stream_releases: Arc::clone(&stream_releases),
    };
    let cache = UpstreamCache::default();
    let target = UpstreamTarget::select(
        &UpstreamResolver::system(),
        &NetworkMechanics::new(),
        Protocol::Tls,
        "brokered.example",
        443,
        &cache,
        Some(&grants),
    )
    .await;
    assert_eq!(target.judged_ip("brokered.example"), Some("127.0.0.1".parse().unwrap()));
    assert_eq!(target.protocol(Protocol::Tls), Protocol::Http);

    let (connected, accepted) = tokio::join!(target.connect_with_grants(Some(&grants)), listener.accept());
    let (stream, pinned) = connected.unwrap();
    let (_peer, _) = accepted.unwrap();
    assert!(matches!(pinned, UpstreamTarget::Granted { selection: None, .. }));
    assert_eq!(selection_releases.load(Ordering::Relaxed), 0);
    drop(stream);
    assert_eq!(stream_releases.load(Ordering::Relaxed), 1);

    let unused = UpstreamTarget::select(
        &UpstreamResolver::system(),
        &NetworkMechanics::new(),
        Protocol::Tls,
        "blocked.example",
        443,
        &UpstreamCache::default(),
        Some(&grants),
    )
    .await;
    drop(unused);
    assert_eq!(selection_releases.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn installed_grants_never_fall_back_to_a_direct_target() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let grants = TestGrants {
        address: listener.local_addr().unwrap(),
        next_id: AtomicU64::new(1),
        selection_releases: Arc::new(AtomicUsize::new(0)),
        stream_releases: Arc::new(AtomicUsize::new(0)),
    };
    let direct = UpstreamTarget::Resolved(vec![listener.local_addr().unwrap()]);
    let error = match direct.connect_with_grants(Some(&grants)).await {
        Ok(_) => panic!("direct connection bypassed installed grants"),
        Err(error) => error,
    };
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(50), listener.accept())
            .await
            .is_err()
    );
}
