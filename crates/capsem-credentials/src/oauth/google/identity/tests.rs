use std::collections::BTreeSet;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use ring::rand::SystemRandom;
use ring::signature::{RsaKeyPair, RSA_PKCS1_SHA256};
use serde_json::{json, Value};

use super::super::super::{LoopbackRedirect, OAuthAttempt, OAuthPolicy};
use super::super::tests::fixture::{Fixture, Reply};
use super::*;

const CLIENT: &str = "fixture.apps.googleusercontent.com";
const SUBJECT: &str = "private-account-subject";
const JWKS: &str = include_str!("../../../../../../tests/fixtures/oauth/identity-jwks-test-only.json");
const KEY: &[u8] = include_bytes!("../../../../../../tests/fixtures/oauth/identity-signing-test-only.der");

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
}

fn signed(header: &str, claims: &str) -> String {
    let pair = RsaKeyPair::from_der(KEY).unwrap();
    let input = format!("{}.{}", URL_SAFE_NO_PAD.encode(header), URL_SAFE_NO_PAD.encode(claims));
    let mut signature = vec![0; pair.public().modulus_len()];
    pair.sign(
        &RSA_PKCS1_SHA256,
        &SystemRandom::new(),
        input.as_bytes(),
        &mut signature,
    )
    .unwrap();
    format!("{input}.{}", URL_SAFE_NO_PAD.encode(signature))
}

fn exchange() -> super::super::super::CallbackExchange {
    let now = Instant::now();
    let mut owner = OAuthAttempt::new(
        LoopbackRedirect::new("127.0.0.1:4123".parse().unwrap(), "/callback").unwrap(),
        OAuthPolicy {
            lifetime: Duration::from_secs(5),
            max_callback_bytes: 2048,
        },
        now,
    )
    .unwrap();
    let state = owner.authorization(now).unwrap().state.to_owned();
    owner
        .accept(&format!("http://127.0.0.1:4123/callback?state={state}&code=code"), now)
        .unwrap()
}

fn claims(nonce: &str) -> Value {
    json!({"iss":"https://accounts.google.com","aud":CLIENT,"sub":SUBJECT,
        "iat":now()-1,"exp":now()+60,"nonce":nonce,"email":"display-only@example.org"})
}

async fn setup(
    jwt: &str,
    exchange: super::super::super::CallbackExchange,
    keys: Reply,
) -> (Fixture, GoogleOAuthClient, OAuthTokens) {
    let token = json!({"access_token":"private-access", "expires_in":60, "scope":"openid",
        "token_type":"Bearer", "id_token":jwt})
    .to_string();
    let fixture = Fixture::new(vec![Reply::json(&token), keys]).await;
    let policy = super::super::OAuthHttpPolicy {
        request_timeout: Duration::from_millis(200),
        max_request_bytes: 4096,
        max_response_bytes: 8192,
    };
    let mut client = GoogleOAuthClient::new(
        super::super::GoogleRegistration::new(CLIENT.into(), None).unwrap(),
        BTreeSet::from(["openid".into()]),
        policy,
    )
    .unwrap();
    client.http = super::super::build_http(policy, false).unwrap();
    client.token_endpoint = format!("{}/token", fixture.base).parse().unwrap();
    client.identity_keys_endpoint = format!("{}/certs", fixture.base).parse().unwrap();
    let tokens = client.exchange(exchange).await.unwrap();
    (fixture, client, tokens)
}

#[tokio::test]
async fn signed_identity_binds_subject_client_nonce_and_fixed_key_fetch() {
    for issuer in ["https://accounts.google.com", "accounts.google.com"] {
        let exchange = exchange();
        let nonce = exchange.nonce().to_owned();
        let mut claims = claims(&nonce);
        claims["iss"] = json!(issuer);
        claims["azp"] = json!(CLIENT);
        let jwt = signed(r#"{"alg":"RS256","kid":"test-only","typ":"JWT"}"#, &claims.to_string());
        let (fixture, client, tokens) = setup(&jwt, exchange, Reply::json(JWKS)).await;
        let identity = client.verify_identity(&tokens, Some(SUBJECT)).await.unwrap();
        assert_eq!(identity.subject(), SUBJECT);
        assert_eq!(identity.expires_at_unix_seconds(), claims["exp"].as_u64().unwrap());
        let debug = format!("{identity:?} {tokens:?}");
        for secret in [&jwt, &nonce, SUBJECT, "display-only@example.org"] {
            assert!(!debug.contains(secret));
        }
        {
            let records = fixture.records.lock().unwrap();
            assert_eq!(records.len(), 2);
            assert_eq!(records[1].method, "GET");
            assert_eq!(records[1].path, "/certs");
            assert!(records[1].fields.is_empty());
            drop(records);
        }
        assert_eq!(client.identity_keys_endpoint.path(), "/certs");
        fixture.close().await;
    }
    let client = GoogleOAuthClient::new(
        super::super::GoogleRegistration::new(CLIENT.into(), None).unwrap(),
        BTreeSet::from(["openid".into()]),
        super::super::OAuthHttpPolicy {
            request_timeout: Duration::from_secs(1),
            max_request_bytes: 4096,
            max_response_bytes: 8192,
        },
    )
    .unwrap();
    assert_eq!(
        client.identity_keys_endpoint.as_str(),
        "https://www.googleapis.com/oauth2/v3/certs"
    );
}

#[tokio::test]
async fn forged_wrong_client_subject_nonce_and_invalid_times_never_establish_identity() {
    for field in ["iss", "aud", "azp", "nonce", "sub", "exp", "iat", "nbf", "at_hash"] {
        let exchange = exchange();
        let mut values = claims(exchange.nonce());
        values[field] = match field {
            "exp" => json!(now()),
            "iat" | "nbf" => json!(now() + 3600),
            "sub" => json!(""),
            _ => json!("wrong-private-value"),
        };
        let jwt = signed(r#"{"alg":"RS256","kid":"test-only"}"#, &values.to_string());
        let (fixture, client, tokens) = setup(&jwt, exchange, Reply::json(JWKS)).await;
        let error = client.verify_identity(&tokens, None).await.unwrap_err();
        assert!(!format!("{error:?} {error}").contains("private"));
        fixture.close().await;
    }
    let exchange = exchange();
    let jwt = signed(
        r#"{"alg":"RS256","kid":"test-only"}"#,
        &claims(exchange.nonce()).to_string(),
    );
    let (fixture, client, tokens) = setup(&jwt, exchange, Reply::json(JWKS)).await;
    assert!(matches!(
        client.verify_identity(&tokens, Some("other-account")).await,
        Err(OAuthIdentityError::AccountMismatch)
    ));
    fixture.close().await;
}

#[tokio::test]
async fn untrusted_header_keys_and_signature_cannot_change_the_verification_authority() {
    for header in [
        r#"{"alg":"HS256","kid":"test-only"}"#,
        r#"{"alg":"RS256","kid":"unknown"}"#,
        r#"{"alg":"RS256"}"#,
        r#"{"alg":"RS256","kid":"test-only","crit":["unknown"]}"#,
        r#"{"alg":"RS256","kid":"test-only","jku":"http://attacker.invalid/keys"}"#,
        r#"{"alg":"RS256","kid":"test-only","alg":"HS256"}"#,
    ] {
        let exchange = exchange();
        let jwt = signed(header, &claims(exchange.nonce()).to_string());
        let (fixture, client, tokens) = setup(&jwt, exchange, Reply::json(JWKS)).await;
        assert!(client.verify_identity(&tokens, None).await.is_err());
        assert!(fixture
            .records
            .lock()
            .unwrap()
            .iter()
            .all(|request| request.path == "/token" || request.path == "/certs"));
        fixture.close().await;
    }
    let exchange = exchange();
    let original = signed(
        r#"{"alg":"RS256","kid":"test-only"}"#,
        &claims(exchange.nonce()).to_string(),
    );
    let (message, signature) = original.rsplit_once('.').unwrap();
    let mut forged = URL_SAFE_NO_PAD.decode(signature).unwrap();
    forged[0] ^= 1;
    let jwt = format!("{message}.{}", URL_SAFE_NO_PAD.encode(forged));
    let (fixture, client, tokens) = setup(&jwt, exchange, Reply::json(JWKS)).await;
    assert!(client.verify_identity(&tokens, None).await.is_err());
    fixture.close().await;
}

#[tokio::test]
async fn missing_duplicate_and_malformed_claims_fail_closed() {
    for missing in ["iss", "aud", "sub", "exp", "iat", "nonce"] {
        let exchange = exchange();
        let mut value = claims(exchange.nonce());
        value.as_object_mut().unwrap().remove(missing);
        let jwt = signed(r#"{"alg":"RS256","kid":"test-only"}"#, &value.to_string());
        let (fixture, client, tokens) = setup(&jwt, exchange, Reply::json(JWKS)).await;
        assert!(client.verify_identity(&tokens, None).await.is_err());
        fixture.close().await;
    }
    for extra in [
        r#", "sub":"duplicate"}"#,
        r#", "exp":"not-numeric"}"#,
        r#", "aud":["fixture.apps.googleusercontent.com"]}"#,
    ] {
        let exchange = exchange();
        let value = claims(exchange.nonce()).to_string();
        let jwt = signed(
            r#"{"alg":"RS256","kid":"test-only"}"#,
            &format!("{}{extra}", &value[..value.len() - 1]),
        );
        let (fixture, client, tokens) = setup(&jwt, exchange, Reply::json(JWKS)).await;
        assert!(client.verify_identity(&tokens, None).await.is_err());
        fixture.close().await;
    }
}

#[tokio::test]
async fn invalid_duplicate_oversized_redirected_and_delayed_jwks_fail_without_replay() {
    let mut duplicate: Value = serde_json::from_str(JWKS).unwrap();
    let key = duplicate["keys"][0].clone();
    duplicate["keys"].as_array_mut().unwrap().push(key);
    for reply in [
        Reply::json("not-json"),
        Reply::json(r#"{"keys":[]}"#),
        Reply::json(&duplicate.to_string()),
        Reply::json(&"x".repeat(9000)),
        Reply {
            status: 307,
            body: String::new(),
            location: Some("http://attacker.invalid/keys".into()),
            delay: Duration::ZERO,
        },
        Reply {
            delay: Duration::from_millis(400),
            ..Reply::json(JWKS)
        },
    ] {
        let exchange = exchange();
        let jwt = signed(
            r#"{"alg":"RS256","kid":"test-only"}"#,
            &claims(exchange.nonce()).to_string(),
        );
        let (fixture, client, tokens) = setup(&jwt, exchange, reply).await;
        assert!(client.verify_identity(&tokens, None).await.is_err());
        assert_eq!(fixture.records.lock().unwrap().len(), 2);
        fixture.close().await;
    }
}

#[tokio::test]
async fn missing_nonce_expired_material_and_client_mismatch_never_fetch_keys() {
    let exchange = exchange();
    let jwt = signed(
        r#"{"alg":"RS256","kid":"test-only"}"#,
        &claims(exchange.nonce()).to_string(),
    );
    let (fixture, mut client, mut tokens) = setup(&jwt, exchange, Reply::json(JWKS)).await;
    let token = tokens.id_token.take();
    assert!(matches!(
        client.verify_identity(&tokens, None).await,
        Err(OAuthIdentityError::NoIdentity)
    ));
    tokens.id_token = token;
    client.registration.client_id = "other-client".into();
    assert!(matches!(
        client.verify_identity(&tokens, None).await,
        Err(OAuthIdentityError::Transport(OAuthTokenError::RegistrationMismatch))
    ));
    client.registration.client_id = CLIENT.into();
    let nonce = tokens.expected_nonce.take();
    assert!(matches!(
        client.verify_identity(&tokens, None).await,
        Err(OAuthIdentityError::NoIdentity)
    ));
    tokens.expected_nonce = nonce;
    tokens.expires_at = Instant::now();
    assert!(matches!(
        client.verify_identity(&tokens, None).await,
        Err(OAuthIdentityError::InvalidIdentity)
    ));
    assert_eq!(fixture.records.lock().unwrap().len(), 1);
    fixture.close().await;
}

#[tokio::test]
async fn optional_access_hash_is_checked_and_jwk_usage_cannot_be_repurposed() {
    let initial_exchange = exchange();
    let mut values = claims(initial_exchange.nonce());
    values["at_hash"] = json!(URL_SAFE_NO_PAD.encode(&Sha256::digest(b"private-access")[..16]));
    let jwt = signed(r#"{"alg":"RS256","kid":"test-only"}"#, &values.to_string());
    let (fixture, client, tokens) = setup(&jwt, initial_exchange, Reply::json(JWKS)).await;
    assert_eq!(client.verify_identity(&tokens, None).await.unwrap().subject(), SUBJECT);
    fixture.close().await;
    for (field, value) in [
        ("alg", json!("HS256")),
        ("kty", json!("oct")),
        ("use", json!("enc")),
        ("key_ops", json!(["sign"])),
        ("n", json!("AQAB")),
        ("e", json!("")),
    ] {
        let exchange = exchange();
        let jwt = signed(
            r#"{"alg":"RS256","kid":"test-only"}"#,
            &claims(exchange.nonce()).to_string(),
        );
        let mut keys: Value = serde_json::from_str(JWKS).unwrap();
        keys["keys"][0][field] = value;
        let (fixture, client, tokens) = setup(&jwt, exchange, Reply::json(&keys.to_string())).await;
        assert!(matches!(
            client.verify_identity(&tokens, None).await,
            Err(OAuthIdentityError::InvalidKeys)
        ));
        fixture.close().await;
    }
}
