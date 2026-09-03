//! Provider binding is a label-boundary suffix match, never a substring one.
//!
//! `evil-openai.com` once bound to OpenAI because `ends_with("openai.com")`
//! is true for it, so a guest that presented the user's brokered OpenAI
//! reference to that host had the real key injected on the way out.

use super::*;

fn seed(provider: CredentialProvider, raw: &str) -> String {
    broker_observed_credential(&CredentialObservation {
        provider,
        raw_value: raw.to_string(),
        source: "http.header.authorization".to_string(),
        event_type: Some("http.request".to_string()),
        trace_id: None,
        context_json: None,
    })
    .unwrap()
    .credential_ref
}

fn bearer(reference: &str) -> http::HeaderMap {
    let mut headers = http::HeaderMap::new();
    headers.insert(
        http::header::AUTHORIZATION,
        http::HeaderValue::from_str(&format!("Bearer {reference}")).unwrap(),
    );
    headers
}

const HOSTILE_OPENAI_HOSTS: &[&str] = &[
    "evil-openai.com",
    "openai.com.evil.example",
    "xopenai.com",
    "EVIL-OPENAI.COM",
    "OPENAI.COM.EVIL.EXAMPLE",
    "api.openai.com..",
    ".openai.com",
    "openai.comx",
    "api.openai.com.evil.example",
    "openai.com\u{2024}evil.example",
];

const LEGITIMATE_OPENAI_HOSTS: &[&str] = &["openai.com", "api.openai.com", "api.openai.com.", "API.OPENAI.COM"];

#[test]
fn host_is_or_under_matches_only_on_label_boundaries() {
    assert!(host_is_or_under("openai.com", "openai.com"));
    assert!(host_is_or_under("api.openai.com", "openai.com"));
    assert!(host_is_or_under("a.b.openai.com", "openai.com"));
    assert!(host_is_or_under("API.OpenAI.com", "openai.com"));
    assert!(
        host_is_or_under("api.openai.com.", "openai.com"),
        "one trailing dot is the FQDN form"
    );

    assert!(!host_is_or_under("evil-openai.com", "openai.com"));
    assert!(!host_is_or_under("xopenai.com", "openai.com"));
    assert!(!host_is_or_under("openai.com.evil.example", "openai.com"));
    assert!(!host_is_or_under("openai.comx", "openai.com"));
    assert!(
        !host_is_or_under("openai.com..", "openai.com"),
        "two trailing dots is not a host"
    );
    assert!(
        !host_is_or_under(".openai.com", "openai.com"),
        "an empty leading label is not a host"
    );
    assert!(!host_is_or_under("", "openai.com"));
    assert!(!host_is_or_under("com", "openai.com"));
    assert!(!host_is_or_under("openai.co", "openai.com"));
}

#[test]
fn provider_binding_rejects_lookalike_domains() {
    for host in HOSTILE_OPENAI_HOSTS {
        assert_eq!(
            credential_provider_for_request(host, None),
            None,
            "{host:?} must not bind to any provider"
        );
    }
    for host in LEGITIMATE_OPENAI_HOSTS {
        assert_eq!(
            credential_provider_for_request(host, None),
            Some(CredentialProvider::OpenAi),
            "{host:?} must bind to OpenAI"
        );
    }
    for (host, provider) in [
        ("evil-anthropic.com", None),
        ("anthropic.com.evil.example", None),
        ("api.anthropic.com", Some(CredentialProvider::Anthropic)),
        ("notclaude.com", None),
        ("claude.com", Some(CredentialProvider::Anthropic)),
        ("evil-googleapis.com", None),
        ("googleapis.com.attacker.net", None),
        ("oauth2.googleapis.com", Some(CredentialProvider::Google)),
        ("mygithub.com", None),
        ("github.com.evil.example", None),
        ("api.github.com", Some(CredentialProvider::Github)),
    ] {
        assert_eq!(credential_provider_for_request(host, None), provider, "{host:?}");
    }
}

#[test]
fn brokered_reference_is_never_dereferenced_for_lookalike_domain() {
    let _lock = TEST_ENV_LOCK.blocking_lock();
    let dir = tempfile::tempdir().unwrap();
    let capsem_home = dir.path().join("capsem-home");
    let test_store = dir.path().join("credential-store.json");
    let _guard = EnvGuard::install(&capsem_home, dir.path(), &test_store);

    let secret = "sk-openai-real-key-do-not-leak";
    let reference = seed(CredentialProvider::OpenAi, secret);

    for host in HOSTILE_OPENAI_HOSTS {
        let mut headers = bearer(&reference);
        let result = substitute_brokered_upstream_credentials(host, None, &mut headers, None);
        assert!(result.is_err(), "{host:?} must not receive the brokered secret");
        let auth = headers[http::header::AUTHORIZATION].to_str().unwrap().to_string();
        assert!(!auth.contains(secret), "{host:?}: plaintext leaked into header: {auth}");
        assert!(
            auth.contains(&reference),
            "{host:?}: reference must stay opaque: {auth}"
        );

        let query = format!("api_key={reference}");
        let result = substitute_brokered_upstream_credentials(host, None, &mut http::HeaderMap::new(), Some(&query));
        assert!(result.is_err(), "{host:?} must not receive the secret via query either");
    }

    for host in LEGITIMATE_OPENAI_HOSTS {
        let mut headers = bearer(&reference);
        substitute_brokered_upstream_credentials(host, None, &mut headers, None)
            .unwrap_or_else(|e| panic!("{host:?} must resolve the OpenAI reference: {e}"));
        assert_eq!(
            headers[http::header::AUTHORIZATION].to_str().unwrap(),
            format!("Bearer {secret}")
        );
    }
}

#[test]
fn injection_ledger_does_not_attribute_lookalike_domains_to_a_provider() {
    let _lock = TEST_ENV_LOCK.blocking_lock();
    let dir = tempfile::tempdir().unwrap();
    let capsem_home = dir.path().join("capsem-home");
    let test_store = dir.path().join("credential-store.json");
    let _guard = EnvGuard::install(&capsem_home, dir.path(), &test_store);
    let reference = seed(CredentialProvider::Anthropic, "sk-ant-ledger-secret");

    let found = detect_brokered_http_references("evil-anthropic.com", None, &bearer(&reference), None, None);
    assert_eq!(found.len(), 1);
    // The reference itself is stored under Anthropic, so the fallback lookup
    // still names the owning provider -- but only through the store, never
    // through the hostile domain.
    assert_eq!(found[0].provider, Some(CredentialProvider::Anthropic));
    assert!(found[0].context_json.as_deref().unwrap().contains("evil-anthropic.com"));
}

#[test]
fn oauth_body_capture_requires_label_boundary() {
    assert!(is_http_body_credential_candidate("oauth2.googleapis.com", "/token"));
    assert!(is_http_body_credential_candidate("oauth2.googleapis.com.", "/token"));
    assert!(is_http_body_credential_candidate("OAUTH2.GOOGLEAPIS.COM", "/token"));
    assert!(!is_http_body_credential_candidate("evil-googleapis.com", "/token"));
    assert!(!is_http_body_credential_candidate(
        "googleapis.com.evil.example",
        "/token"
    ));
    assert!(is_http_body_credential_candidate(
        "github.com",
        "/login/oauth/access_token"
    ));
    assert!(!is_http_body_credential_candidate(
        "evil-github.com",
        "/login/oauth/access_token"
    ));

    // A lookalike host never captures a body field as a provider credential.
    let body = br#"{"access_token":"opaque-token-value"}"#;
    assert!(detect_http_body_credentials("evil-googleapis.com", "/token", "response", body).is_empty());
    assert!(
        detect_http_body_credentials("github.com.evil.example", "/login/oauth/access_token", "response", body)
            .is_empty()
    );
    assert_eq!(
        detect_http_body_credentials("oauth2.googleapis.com", "/token", "response", body).len(),
        1
    );

    // GitHub's opaque-token header capture is bound to github.com, not to any
    // host ending in those letters.
    assert_eq!(
        provider_for_token("evil-github.com", "authorization", "opaque-value"),
        None
    );
    assert_eq!(
        provider_for_token("api.github.com", "authorization", "opaque-value"),
        Some(CredentialProvider::Github)
    );
}
