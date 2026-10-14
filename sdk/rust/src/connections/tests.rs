use crate::{models, test_gateway::Server, Error, Hypervisor, VmSelector};
use serde_json::{json, Value};

const REQUEST: &str = "fc120782-2272-4b3d-a2e6-a26ee2f8ab61";
const OWNER: &str = "bb120782-2272-4b3d-a2e6-a26ee2f8ab61";
const CONNECTION: &str = "connection /?&é";
const SESSION: &str = "session /?&é";
const GRANT: &str = "grant /?&é";
const TRANSACTION: &str = "transaction /?&é";

fn connection() -> Value {
    json!({"id":CONNECTION,"provider":"google","scope_profile_id":"agy-native",
        "account_label":null,"state":"connected","revision":1,"granted_scopes":["openid"],
        "available_capabilities":["agy"],"created_at":"2026-10-14T00:00:00Z",
        "updated_at":"2026-10-14T00:00:00Z","last_refresh_at":null,
        "provider_revocation":"not_started","error_code":null})
}

fn transaction() -> Value {
    json!({"id":TRANSACTION,"state":"pending","expires_at":"2026-10-14T01:00:00Z",
        "connection_id":null,"error_code":null})
}

fn grant() -> Value {
    json!({"id":GRANT,"session_id":SESSION,"owner_generation":OWNER,"connection_id":CONNECTION,
        "capability_ids":["agy"],"state":"active","revision":1,"created_at":"2026-10-14T00:00:00Z",
        "expires_at":"2026-10-14T01:00:00Z","revoked_at":null,"runtime_bound":true})
}

fn authorize_request() -> models::AuthorizeGoogleRequest {
    models::AuthorizeGoogleRequest {
        request_id: REQUEST.into(),
        scope_profile_id: "agy-native".into(),
        reconnect_connection_id: Some(CONNECTION.into()),
    }
}

fn grant_request() -> models::CreateSessionGrantRequest {
    models::CreateSessionGrantRequest {
        request_id: REQUEST.into(),
        expected_owner_generation: OWNER.into(),
        connection_id: CONNECTION.into(),
        capability_ids: vec!["agy".into()],
        expires_at: None,
    }
}

#[tokio::test]
async fn all_nine_calls_use_authenticated_exact_paths_and_explicit_mutations() {
    let responses = [
        (
            200,
            json!({"connections":[connection()],"scope_profiles":[],"next_cursor":null}),
        ),
        (
            202,
            json!({"transaction":transaction(),"authorization_url":"https://accounts.google.com/consent"}),
        ),
        (200, transaction()),
        (200, {
            let mut value = transaction();
            value["state"] = json!("cancelled");
            value
        }),
        (200, connection()),
        (
            202,
            json!({"connection":connection(),"revoked_grant_count":1,"local_authority":"revoking"}),
        ),
        (
            200,
            json!({"session_id":SESSION,"owner_generation":OWNER,"grants":[grant()],"denied_capability_ids":[]}),
        ),
        (201, grant()),
        (200, {
            let mut revoked = grant();
            revoked["state"] = json!("revoked");
            revoked["revoked_at"] = json!("2026-10-14T00:01:00Z");
            json!({"grant":revoked,"local_authority":"revoked"})
        }),
    ];
    let encoded_connection = "connection%20%2F%3F&%C3%A9";
    let encoded_session = "session%20%2F%3F&%C3%A9";
    let encoded_transaction = "transaction%20%2F%3F&%C3%A9";
    let paths = [
        "/connections".into(),
        "/connections/google/authorize".into(),
        format!("/connections/authorizations/{encoded_transaction}"),
        format!("/connections/authorizations/{encoded_transaction}"),
        format!("/connections/{encoded_connection}"),
        format!("/connections/{encoded_connection}"),
        format!("/vms/{encoded_session}/credential-grants"),
        format!("/vms/{encoded_session}/credential-grants"),
        format!("/vms/{encoded_session}/credential-grants/grant%20%2F%3F&%C3%A9"),
    ];
    for (index, (status, response)) in responses.into_iter().enumerate() {
        let mut server = Server::reply(status, response.to_string().as_bytes(), None).await;
        let hv = Hypervisor::new(&server.url, "private-token").unwrap();
        let vm = hv.vm(VmSelector::Id(SESSION.into())).unwrap();
        let body = match index {
            0 => {
                assert_eq!(hv.connections().list().await.unwrap().connections[0].id, CONNECTION);
                None
            }
            1 => {
                assert_eq!(
                    hv.connections()
                        .authorize_google(authorize_request())
                        .await
                        .unwrap()
                        .transaction
                        .id,
                    TRANSACTION
                );
                Some(json!(authorize_request()))
            }
            2 => {
                assert_eq!(
                    hv.connections().authorization(TRANSACTION).await.unwrap().state,
                    models::TransactionState::Pending
                );
                None
            }
            3 => {
                assert_eq!(
                    hv.connections().cancel_authorization(TRANSACTION).await.unwrap().state,
                    models::TransactionState::Cancelled
                );
                None
            }
            4 => {
                assert_eq!(hv.connections().get(CONNECTION).await.unwrap().id, CONNECTION);
                None
            }
            5 => {
                assert_eq!(
                    hv.connections().disconnect(CONNECTION).await.unwrap().local_authority,
                    models::LocalCredentialAuthority::Revoking
                );
                None
            }
            6 => {
                assert_eq!(vm.credential_grants().list().await.unwrap().grants[0].id, GRANT);
                None
            }
            7 => {
                assert_eq!(
                    vm.credential_grants()
                        .create(grant_request())
                        .await
                        .unwrap()
                        .owner_generation,
                    OWNER
                );
                Some(json!(grant_request()))
            }
            8 => {
                assert_eq!(
                    vm.credential_grants().revoke(GRANT).await.unwrap().local_authority,
                    models::RevokedCredentialAuthority::Revoked
                );
                None
            }
            _ => unreachable!(),
        };
        let (parts, bytes) = server.received.recv().await.unwrap();
        assert_eq!(
            parts.method.as_str(),
            ["GET", "POST", "GET", "DELETE", "GET", "DELETE", "GET", "POST", "DELETE"][index]
        );
        assert_eq!(parts.uri.to_string(), paths[index]);
        assert_eq!(parts.headers["authorization"], "Bearer private-token");
        match body {
            Some(body) => assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap(), body),
            None => assert!(bytes.is_empty()),
        }
        assert!(
            server.received.try_recv().is_err(),
            "no automatic polling, grant or replay"
        );
    }
}

#[tokio::test]
async fn refusal_preserves_status_and_typed_code_without_replaying_mutation() {
    let body = br#"{"error":"session ownership changed","code":"owner_changed"}"#;
    let mut server = Server::reply(409, body, None).await;
    let hv = Hypervisor::new(&server.url, "private-token").unwrap();
    let vm = hv.vm(VmSelector::Id(SESSION.into())).unwrap();
    let error = vm.credential_grants().create(grant_request()).await.unwrap_err();
    assert_eq!(error.credential_code(), Some(models::CredentialErrorCode::OwnerChanged));
    assert!(matches!(error, Error::Http { status: 409, .. }));
    server.received.recv().await.unwrap();
    assert!(server.received.try_recv().is_err());
}

#[tokio::test]
async fn grant_responses_cannot_confirm_a_different_session_or_owner() {
    for field in ["session_id", "owner_generation", "connection_id", "capability_ids"] {
        let mut response = grant();
        response[field] = if field == "capability_ids" {
            json!(["other-consumer"])
        } else {
            json!(REQUEST)
        };
        let mut server = Server::reply(201, response.to_string().as_bytes(), None).await;
        let hv = Hypervisor::new(&server.url, "private-token").unwrap();
        let vm = hv.vm(VmSelector::Id(SESSION.into())).unwrap();
        assert!(matches!(
            vm.credential_grants().create(grant_request()).await,
            Err(Error::InvalidResponse(_))
        ));
        server.received.recv().await.unwrap();
        assert!(server.received.try_recv().is_err());
    }
}

#[tokio::test]
async fn reads_and_revoke_reject_substituted_identities() {
    for index in 0..4 {
        let response = match index {
            0 => {
                let mut value = connection();
                value["id"] = json!("other");
                value
            }
            1 => {
                let mut value = transaction();
                value["id"] = json!("other");
                value
            }
            2 => json!({"session_id":"other","owner_generation":OWNER,"grants":[],"denied_capability_ids":[]}),
            3 => {
                let mut value = grant();
                value["id"] = json!("other");
                json!({"grant":value,"local_authority":"revoked"})
            }
            _ => unreachable!(),
        };
        let mut server = Server::reply(200, response.to_string().as_bytes(), None).await;
        let hv = Hypervisor::new(&server.url, "private-token").unwrap();
        let vm = hv.vm(VmSelector::Id(SESSION.into())).unwrap();
        let error = match index {
            0 => hv.connections().get(CONNECTION).await.unwrap_err(),
            1 => hv.connections().authorization(TRANSACTION).await.unwrap_err(),
            2 => vm.credential_grants().list().await.unwrap_err(),
            3 => vm.credential_grants().revoke(GRANT).await.unwrap_err(),
            _ => unreachable!(),
        };
        assert!(matches!(error, Error::InvalidResponse(_)));
        server.received.recv().await.unwrap();
        assert!(server.received.try_recv().is_err());
    }
}

#[tokio::test]
async fn consent_and_grants_keep_the_callers_finite_body_deadline() {
    for consent in [true, false] {
        let mut server = Server::respond(|_| {
            let stream = futures::stream::pending::<std::result::Result<axum::body::Bytes, std::io::Error>>();
            axum::http::Response::builder()
                .status(202)
                .body(axum::body::Body::from_stream(stream))
                .unwrap()
        })
        .await;
        let hv = Hypervisor::new(&server.url, "private-token")
            .unwrap()
            .with_timeout(std::time::Duration::from_millis(50))
            .unwrap();
        let error = if consent {
            hv.connections()
                .authorize_google(authorize_request())
                .await
                .unwrap_err()
        } else {
            hv.vm(VmSelector::Id(SESSION.into()))
                .unwrap()
                .credential_grants()
                .create(grant_request())
                .await
                .unwrap_err()
        };
        assert!(matches!(error,Error::Transport(error) if error.is_timeout()));
        server.received.recv().await.unwrap();
        assert!(server.received.try_recv().is_err());
    }
}

#[test]
fn credential_codes_ignore_unknown_and_non_json_envelopes() {
    for body in [
        b"not JSON".as_slice(),
        br#"{"code":"future_provider_code"}"#,
        br#"{"error":"unavailable"}"#,
    ] {
        assert_eq!(
            Error::Http {
                status: 503,
                body: body.to_vec()
            }
            .credential_code(),
            None
        );
    }
    assert_eq!(Error::InvalidResponse("invalid identity").credential_code(), None);
}
