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
    epoch.register(GatewayAuthSecret::from_bytes([b'A'; 64])).unwrap();
    assert_eq!(
        authority.authenticate_gateway(&headers(&token)).unwrap().evidence(),
        &capsem_api::CallerActor::Operator
    );
    assert!(authority.authenticate_gateway(&headers(&"B".repeat(64))).is_err());
    assert!(authority.authenticate_gateway(&HeaderMap::new()).is_err());
    assert_eq!(
        epoch.register(GatewayAuthSecret::from_bytes([b'B'; 64])),
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
    old.register(GatewayAuthSecret::from_bytes([b'A'; 64])).unwrap();
    let mut new = authority.begin();
    new.register(GatewayAuthSecret::from_bytes([b'B'; 64])).unwrap();
    assert!(authority.authenticate_gateway(&headers(&"A".repeat(64))).is_err());
    drop(old);
    assert!(authority.authenticate_gateway(&headers(&"B".repeat(64))).is_ok());
    drop(authority.begin());
    assert!(authority.authenticate_gateway(&headers(&"B".repeat(64))).is_ok());
    drop(new);
    assert!(authority.authenticate_gateway(&headers(&"B".repeat(64))).is_err());
}
