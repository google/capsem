use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use super::*;
use crate::{LoopbackRedirect, OAuthAttempt, OAuthPolicy};

mod fixture;
use fixture::{Fixture, Reply};

const TOKEN: &str = r#"{"access_token":"private-access","refresh_token":"private-refresh","id_token":"private-id","expires_in":60,"refresh_token_expires_in":120,"scope":"scope.read","token_type":"Bearer","extra":"ignored"}"#;

fn policy() -> OAuthHttpPolicy {
    OAuthHttpPolicy {
        request_timeout: Duration::from_millis(200),
        max_request_bytes: 4096,
        max_response_bytes: 4096,
    }
}

fn scopes() -> BTreeSet<String> {
    BTreeSet::from(["scope.read".into(), "scope.write".into()])
}

fn client(fixture: &Fixture) -> GoogleOAuthClient {
    let mut client = GoogleOAuthClient::new(
        GoogleRegistration::new(
            "fixture.apps.googleusercontent.com".into(),
            Some("private-client-secret".into()),
        )
        .unwrap(),
        scopes(),
        policy(),
    )
    .unwrap();
    client.http = build_http(policy(), false).unwrap();
    client.token_endpoint = format!("{}/token", fixture.base).parse().unwrap();
    client.revoke_endpoint = format!("{}/revoke", fixture.base).parse().unwrap();
    client
}

fn exchange(code: &str) -> CallbackExchange {
    let now = Instant::now();
    let redirect = LoopbackRedirect::new("127.0.0.1:4123".parse().unwrap(), "/callback").unwrap();
    let mut attempt = OAuthAttempt::new(
        redirect,
        OAuthPolicy {
            lifetime: Duration::from_secs(5),
            max_callback_bytes: 8192,
        },
        now,
    )
    .unwrap();
    let state = attempt.authorization(now).unwrap().state.to_string();
    let encoded = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("code", code)
        .finish();
    attempt
        .accept(&format!("http://127.0.0.1:4123/callback?state={state}&{encoded}"), now)
        .unwrap()
}

#[tokio::test]
async fn exact_exchange_refresh_and_revoke_forms_keep_secrets_out_of_urls_and_debug() {
    let fixture = Fixture::new(vec![
        Reply::json(TOKEN),
        Reply::json(r#"{"access_token":"private-next","expires_in":60,"token_type":"Bearer"}"#),
        Reply::json(""),
    ])
    .await;
    let client = client(&fixture);
    let code = exchange("private+code &value");
    let verifier = code.verifier().to_string();
    let tokens = client.exchange(code).await.unwrap();
    assert_eq!(tokens.access_token(Instant::now()), Some("private-access"));
    assert_eq!(tokens.refresh_token(), Some("private-refresh"));
    assert_eq!(tokens.unverified_id_token(), Some("private-id"));
    assert_eq!(tokens.granted_scopes(), &BTreeSet::from(["scope.read".into()]));
    assert_eq!(tokens.access_token(tokens.expires_at()), None);
    let refreshed = client.refresh(&tokens).await.unwrap();
    assert_eq!(refreshed.access_token(Instant::now()), Some("private-next"));
    assert_eq!(refreshed.refresh_token(), Some("private-refresh"));
    assert_eq!(refreshed.refresh_expires_at(), tokens.refresh_expires_at());
    assert_eq!(refreshed.granted_scopes(), tokens.granted_scopes());
    client.revoke(&refreshed).await.unwrap();
    {
        let records = fixture.records.lock().unwrap();
        assert_eq!(records.len(), 3);
        for record in records.iter() {
            assert_eq!(record.method, "POST");
            assert_eq!(record.content_type, "application/x-www-form-urlencoded");
            assert!(!record.path.contains('?'));
            assert_eq!(record.field_count, record.fields.len());
        }
        assert_eq!(records[0].path, "/token");
        assert_eq!(
            records[0].fields,
            std::collections::BTreeMap::from([
                ("grant_type".into(), "authorization_code".into()),
                ("client_id".into(), "fixture.apps.googleusercontent.com".into()),
                ("client_secret".into(), "private-client-secret".into()),
                ("code".into(), "private+code &value".into()),
                ("code_verifier".into(), verifier.clone()),
                ("redirect_uri".into(), "http://127.0.0.1:4123/callback".into()),
            ])
        );
        assert_eq!(
            records[1].fields,
            std::collections::BTreeMap::from([
                ("grant_type".into(), "refresh_token".into()),
                ("client_id".into(), "fixture.apps.googleusercontent.com".into()),
                ("client_secret".into(), "private-client-secret".into()),
                ("refresh_token".into(), "private-refresh".into()),
                ("scope".into(), "scope.read".into()),
            ])
        );
        assert_eq!(records[2].path, "/revoke");
        assert_eq!(
            records[2].fields,
            std::collections::BTreeMap::from([("token".into(), "private-refresh".into())])
        );
        drop(records);
    }
    for debug in [format!("{client:?}"), format!("{tokens:?}"), format!("{refreshed:?}")] {
        for secret in [
            &verifier,
            "private-client-secret",
            "private-access",
            "private-refresh",
            "private-id",
            "private-next",
            "private+code &value",
        ] {
            assert!(!debug.contains(secret));
        }
    }
    fixture.close().await;
}

#[tokio::test]
async fn refresh_rotation_is_detached_and_wrong_registration_never_posts() {
    let fixture = Fixture::new(vec![Reply::json(TOKEN), Reply::json(r#"{"access_token":"new-access","refresh_token":"rotated-refresh","expires_in":60,"scope":"scope.read","token_type":"bearer"}"#)]).await;
    let client = client(&fixture);
    let original = client.exchange(exchange("code")).await.unwrap();
    let rotated = client.refresh(&original).await.unwrap();
    assert_eq!(original.refresh_token(), Some("private-refresh"));
    assert_eq!(rotated.refresh_token(), Some("rotated-refresh"));
    let mut wrong = GoogleOAuthClient::new(
        GoogleRegistration::new("other.apps.googleusercontent.com".into(), None).unwrap(),
        scopes(),
        policy(),
    )
    .unwrap();
    wrong.http = build_http(policy(), false).unwrap();
    wrong.token_endpoint = format!("{}/token", fixture.base).parse().unwrap();
    assert!(matches!(
        wrong.refresh(&original).await,
        Err(OAuthTokenError::RegistrationMismatch)
    ));
    assert!(matches!(
        wrong.revoke(&original).await,
        Err(OAuthTokenError::RegistrationMismatch)
    ));
    assert_eq!(fixture.records.lock().unwrap().len(), 2);
    fixture.close().await;
}

#[tokio::test]
async fn malformed_duplicate_broadened_expired_and_oversized_responses_refuse_without_echo() {
    for body in [
        "not-json",
        r#"{"access_token":"one","access_token":"two","expires_in":60,"scope":"scope.read","token_type":"Bearer"}"#,
        r#"{"access_token":"private-access","expires_in":0,"scope":"scope.read","token_type":"Bearer"}"#,
        r#"{"access_token":"private-access","expires_in":60,"scope":"scope.admin","token_type":"Bearer"}"#,
        r#"{"access_token":"private-access","expires_in":60,"scope":"scope.read","token_type":"other"}"#,
        r#"{"access_token":"","expires_in":60,"scope":"scope.read","token_type":"Bearer"}"#,
    ] {
        let fixture = Fixture::new(vec![Reply::json(body)]).await;
        let error = client(&fixture).exchange(exchange("private-code")).await.unwrap_err();
        assert!(matches!(
            error,
            OAuthTokenError::InvalidResponse | OAuthTokenError::ScopeDenied
        ));
        assert!(!error.to_string().contains(body));
        fixture.close().await;
    }
    let fixture = Fixture::new(vec![Reply::json(&"x".repeat(8192))]).await;
    assert!(matches!(
        client(&fixture).exchange(exchange("private-code")).await,
        Err(OAuthTokenError::ResponseTooLarge)
    ));
    fixture.close().await;
}

#[tokio::test]
async fn provider_errors_and_redirects_are_redacted_and_never_replayed() {
    let fixture = Fixture::new(vec![Reply {
        status: 400,
        body: r#"{"error":"invalid_grant","error_description":"private-code private-refresh"}"#.into(),
        location: None,
        delay: Duration::ZERO,
    }])
    .await;
    let error = client(&fixture).exchange(exchange("private-code")).await.unwrap_err();
    assert!(matches!(error, OAuthTokenError::ProviderRejected { status: 400, .. }));
    assert!(!format!("{error:?} {error}").contains("private-"));
    assert_eq!(fixture.records.lock().unwrap().len(), 1);
    fixture.close().await;
    let fixture = Fixture::new(vec![Reply {
        status: 307,
        body: String::new(),
        location: Some("/leak?token=private-code".into()),
        delay: Duration::ZERO,
    }])
    .await;
    assert!(matches!(
        client(&fixture).exchange(exchange("private-code")).await,
        Err(OAuthTokenError::ProviderRejected { status: 307, .. })
    ));
    assert_eq!(fixture.records.lock().unwrap().len(), 1);
    fixture.close().await;
}

#[tokio::test]
async fn timeout_and_cancellation_send_once_and_do_not_retry_mutations() {
    let fixture = Fixture::new(vec![Reply {
        delay: Duration::from_millis(400),
        ..Reply::json(TOKEN)
    }])
    .await;
    assert!(matches!(
        client(&fixture).exchange(exchange("private-code")).await,
        Err(OAuthTokenError::Timeout)
    ));
    assert_eq!(fixture.records.lock().unwrap().len(), 1);
    fixture.close().await;
    let fixture = Fixture::new(vec![Reply {
        delay: Duration::from_millis(400),
        ..Reply::json(TOKEN)
    }])
    .await;
    let client = client(&fixture);
    let pending = tokio::spawn(async move { client.exchange(exchange("private-code")).await });
    tokio::time::timeout(Duration::from_secs(2), fixture.received.notified())
        .await
        .unwrap();
    pending.abort();
    assert!(pending.await.unwrap_err().is_cancelled());
    assert_eq!(fixture.records.lock().unwrap().len(), 1);
    fixture.close().await;
}

#[test]
fn authorization_url_uses_explicit_profile_and_debug_hides_state() {
    let client = GoogleOAuthClient::new(
        GoogleRegistration::new("fixture.apps.googleusercontent.com".into(), None).unwrap(),
        scopes(),
        policy(),
    )
    .unwrap();
    let now = Instant::now();
    let mut owner = OAuthAttempt::new(
        LoopbackRedirect::new("127.0.0.1:4123".parse().unwrap(), "/callback").unwrap(),
        OAuthPolicy {
            lifetime: Duration::from_secs(5),
            max_callback_bytes: 1024,
        },
        now,
    )
    .unwrap();
    let params = owner.authorization(now).unwrap();
    let state = params.state.to_string();
    let url = client.authorization_url(params).unwrap();
    let parsed = url::Url::parse(url.as_str()).unwrap();
    assert_eq!(parsed.host_str(), Some("accounts.google.com"));
    let values = parsed
        .query_pairs()
        .into_owned()
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(values["client_id"], "fixture.apps.googleusercontent.com");
    assert_eq!(values["scope"], "scope.read scope.write");
    assert_eq!(values["code_challenge_method"], "S256");
    assert_eq!(values["state"], state);
    assert!(!format!("{url:?}").contains(&state));
}

#[test]
fn registration_and_policy_require_valid_explicit_bounded_values() {
    for id in ["", "client\nsecret", "client id"] {
        assert!(matches!(
            GoogleRegistration::new(id.into(), None),
            Err(OAuthTokenError::InvalidRegistration)
        ));
    }
    for invalid in [
        OAuthHttpPolicy {
            request_timeout: Duration::ZERO,
            ..policy()
        },
        OAuthHttpPolicy {
            request_timeout: Duration::MAX,
            ..policy()
        },
        OAuthHttpPolicy {
            max_request_bytes: 0,
            ..policy()
        },
        OAuthHttpPolicy {
            max_response_bytes: 0,
            ..policy()
        },
    ] {
        assert!(matches!(
            GoogleOAuthClient::new(
                GoogleRegistration::new("fixture".into(), None).unwrap(),
                scopes(),
                invalid
            ),
            Err(OAuthTokenError::InvalidPolicy)
        ));
    }
    assert!(matches!(
        GoogleOAuthClient::new(
            GoogleRegistration::new("fixture".into(), None).unwrap(),
            BTreeSet::new(),
            policy()
        ),
        Err(OAuthTokenError::InvalidRegistration)
    ));
    let oversized = GoogleRegistration::new("fixture".into(), Some("x".repeat(8192))).unwrap();
    assert!(matches!(
        GoogleOAuthClient::new(oversized, scopes(), policy()),
        Err(OAuthTokenError::RequestTooLarge)
    ));
}

#[tokio::test]
async fn refresh_cannot_broaden_scopes_and_identity_aliases_remain_equivalent() {
    let fixture = Fixture::new(vec![
        Reply::json(TOKEN),
        Reply::json(
            r#"{"access_token":"private-next","expires_in":60,"scope":"scope.read scope.write","token_type":"Bearer"}"#,
        ),
    ])
    .await;
    let client = client(&fixture);
    let original = client.exchange(exchange("code")).await.unwrap();
    assert!(matches!(
        client.refresh(&original).await,
        Err(OAuthTokenError::ScopeDenied)
    ));
    assert_eq!(original.granted_scopes(), &BTreeSet::from(["scope.read".into()]));
    fixture.close().await;
    let fixture = Fixture::new(vec![Reply::json(r#"{"access_token":"private-access","expires_in":60,"scope":"https://www.googleapis.com/auth/userinfo.email profile","token_type":"Bearer"}"#)]).await;
    let mut client = GoogleOAuthClient::new(
        GoogleRegistration::new("fixture".into(), None).unwrap(),
        BTreeSet::from([
            "email".into(),
            "https://www.googleapis.com/auth/userinfo.profile".into(),
        ]),
        policy(),
    )
    .unwrap();
    client.http = build_http(policy(), false).unwrap();
    client.token_endpoint = format!("{}/token", fixture.base).parse().unwrap();
    let tokens = client.exchange(exchange("code")).await.unwrap();
    assert_eq!(
        tokens.granted_scopes(),
        &BTreeSet::from([
            "https://www.googleapis.com/auth/userinfo.email".into(),
            "https://www.googleapis.com/auth/userinfo.profile".into()
        ])
    );
    assert!(!fixture.records.lock().unwrap()[0].fields.contains_key("client_secret"));
    fixture.close().await;
}

#[tokio::test]
async fn request_budget_and_missing_or_expired_refresh_refuse_before_posting() {
    let fixture = Fixture::new(vec![Reply::json(
        r#"{"access_token":"private-access","expires_in":60,"scope":"scope.read","token_type":"Bearer"}"#,
    )])
    .await;
    let mut client = client(&fixture);
    client.policy.max_request_bytes = 256;
    assert!(matches!(
        client.exchange(exchange(&"x".repeat(1000))).await,
        Err(OAuthTokenError::RequestTooLarge)
    ));
    assert!(fixture.records.lock().unwrap().is_empty());
    let mut tokens = client.exchange(exchange("code")).await.unwrap();
    assert!(matches!(
        client.refresh(&tokens).await,
        Err(OAuthTokenError::MissingRefreshToken)
    ));
    tokens.refresh_token = Some(Secret::new("private-refresh".into()));
    tokens.refresh_expires_at = Some(Instant::now());
    assert!(matches!(
        client.refresh(&tokens).await,
        Err(OAuthTokenError::ReauthorizationRequired)
    ));
    assert_eq!(fixture.records.lock().unwrap().len(), 1);
    fixture.close().await;
}
