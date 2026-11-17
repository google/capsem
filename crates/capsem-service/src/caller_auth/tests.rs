use super::*;

fn headers(token: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::AUTHORIZATION,
        format!("Bearer {token}").parse().unwrap(),
    );
    headers
}

#[test]
fn only_live_registered_operator_epoch_mints_actor_evidence() {
    let authority = Arc::new(Authority::default());
    let token = "A".repeat(64);
    assert!(authority.authenticate_gateway(&headers(&token)).is_err());
    let mut epoch = authority.begin();
    epoch.activate(GatewayAuthSecret::from_bytes([b'A'; 64])).unwrap();
    assert_eq!(
        authority.authenticate_gateway(&headers(&token)).unwrap().evidence(),
        &capsem_api::CallerActor::Operator
    );
    assert!(authority.authenticate_gateway(&headers(&"B".repeat(64))).is_err());
    assert!(authority.authenticate_gateway(&HeaderMap::new()).is_err());
    assert_eq!(
        epoch.activate(GatewayAuthSecret::from_bytes([b'B'; 64])),
        Err(GatewayAuthDenial::Forbidden)
    );
    assert!(authority.authenticate_gateway(&headers(&token)).is_ok());
    drop(epoch);
    assert!(authority.authenticate_gateway(&headers(&token)).is_err());
}

#[test]
fn old_epoch_drop_cannot_revoke_a_newly_registered_operator() {
    let authority = Arc::new(Authority::default());
    let mut old = authority.begin();
    old.activate(GatewayAuthSecret::from_bytes([b'A'; 64])).unwrap();
    let mut new = authority.begin();
    new.activate(GatewayAuthSecret::from_bytes([b'B'; 64])).unwrap();
    assert!(authority.authenticate_gateway(&headers(&"A".repeat(64))).is_err());
    drop(old);
    assert!(authority.authenticate_gateway(&headers(&"B".repeat(64))).is_ok());
    drop(authority.begin());
    assert!(authority.authenticate_gateway(&headers(&"B".repeat(64))).is_ok());
    drop(new);
    assert!(authority.authenticate_gateway(&headers(&"B".repeat(64))).is_err());
}

#[test]
fn delegated_bearer_requires_issued_live_capability_and_preserves_its_scope() {
    use capsem_core::{
        agent_access::Reservation,
        managed_sessions::{Capability, LeaseClock},
    };
    let authority = Authority::default();
    let id = capsem_crypto::uuid_v4();
    let token = format!("{id}.{}", "11".repeat(32));
    let secret = Capability::from_bytes([0x11; 32]);
    assert!(authority.authenticate_gateway(&headers(&token)).is_err());
    let scope = capsem_api::AgentAccessScope::new(vec![capsem_api::AgentOperation::Discover], vec![], vec![]).unwrap();
    let Reservation::New(admission) = authority
        .agents
        .reserve(id, &secret, scope.clone(), std::time::Duration::from_secs(60), || {
            LeaseClock::now().ok()
        })
        .unwrap()
    else {
        panic!("new grant")
    };
    assert!(authority.authenticate_gateway(&headers(&token)).is_err());
    admission.commit(|| LeaseClock::now().ok()).unwrap();
    let actor = authority.authenticate_gateway(&headers(&token)).unwrap();
    assert_eq!(
        actor.evidence(),
        &capsem_api::CallerActor::DelegatedAgent {
            access_id: id.to_string()
        }
    );
    assert_eq!(actor.agent_grant().unwrap().scope(), &scope);
    assert!(actor.require_operator().is_err());
    assert!(authority.authenticate_gateway(&headers(&"11".repeat(32))).is_err());
    assert!(authority
        .authenticate_gateway(&headers(&format!("{id}.{}", "22".repeat(32))))
        .is_err());
    authority.agents.revoke(id).unwrap();
    assert!(authority.authenticate_gateway(&headers(&token)).is_err());
    assert!(!actor.agent_grant().unwrap().is_live(|| LeaseClock::now().ok()));
}

#[test]
fn competing_bearer_headers_are_refused_before_identity_is_chosen() {
    let authority = fixture_authority();
    let mut competing = headers(&"A".repeat(64));
    competing.append(
        axum::http::header::AUTHORIZATION,
        "Bearer another-identity".parse().unwrap(),
    );
    assert!(authority.authenticate_gateway(&competing).is_err());
}
