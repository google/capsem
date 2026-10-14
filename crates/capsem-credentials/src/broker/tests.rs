use super::*;
mod lifecycle;
mod policy;

fn limits() -> BrokerLimits {
    BrokerLimits {
        max_connections: 8,
        max_transactions: 8,
        max_grants: 8,
        enrollment_lifetime: Duration::from_secs(1),
        grant_lifetime: Duration::from_secs(30),
        retirement_timeout: Duration::from_secs(1),
        ..BrokerLimits::bundled().unwrap()
    }
}

fn authority() -> Arc<crate::GrantAuthority> {
    Arc::new(
        crate::GrantAuthority::new(crate::GrantLimits {
            max_grants: 8,
            max_lifetime: Duration::from_secs(30),
        })
        .unwrap(),
    )
}

fn request() -> api::AuthorizeGoogleRequest {
    api::AuthorizeGoogleRequest {
        request_id: capsem_crypto::uuid_v4().to_string(),
        scope_profile_id: "agy-native".into(),
        reconnect_connection_id: None,
    }
}

#[tokio::test]
async fn unconfigured_native_enrollment_is_honest_and_never_creates_authority() {
    let summary = api::ScopeProfileSummary {
        id: "agy-native".into(),
        enrollment_kind: api::EnrollmentKind::NativeConsumerSignIn,
        label: "Antigravity".into(),
        application_label: "Google Antigravity".into(),
        requested_scopes: vec![],
        capabilities: vec![],
        enabled: false,
    };
    let broker = ConnectionBroker::open(
        BrokerStorage::Memory,
        vec![BrokerProfile::unconfigured(summary).unwrap()],
        authority(),
        limits(),
    )
    .await
    .unwrap();
    let catalog = broker.list().await.unwrap();
    assert!(catalog.connections.is_empty());
    assert!(!catalog.scope_profiles[0].enabled);
    assert_eq!(
        broker.authorize(request()).await.unwrap_err(),
        api::CredentialErrorCode::ConfigurationRequired
    );
    assert!(broker.list().await.unwrap().connections.is_empty());
    assert_eq!(
        broker.authorization("missing").await.unwrap_err(),
        api::CredentialErrorCode::TransactionExpired
    );
}
