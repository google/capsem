use super::*;

struct EnvGuard {
    old_home_override: Option<String>,
    old_home: Option<String>,
    old_store: Option<String>,
}

impl EnvGuard {
    fn install(
        capsem_home: &std::path::Path,
        home: &std::path::Path,
        test_store: &std::path::Path,
    ) -> Self {
        let old_home_override = std::env::var("CAPSEM_HOME").ok();
        let old_home = std::env::var("HOME").ok();
        let old_store = std::env::var(TEST_STORE_ENV).ok();
        std::env::set_var("CAPSEM_HOME", capsem_home);
        std::env::set_var("HOME", home);
        std::env::set_var(TEST_STORE_ENV, test_store);
        Self {
            old_home_override,
            old_home,
            old_store,
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.old_home_override {
            Some(v) => std::env::set_var("CAPSEM_HOME", v),
            None => std::env::remove_var("CAPSEM_HOME"),
        }
        match &self.old_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        match &self.old_store {
            Some(v) => std::env::set_var(TEST_STORE_ENV, v),
            None => std::env::remove_var(TEST_STORE_ENV),
        }
    }
}

#[test]
fn env_parser_detects_ai_and_github_credentials() {
    let found = parse_env_credentials(
        "/workspace/.env",
        r#"
        OPENAI_API_KEY="sk-test-openai"
        GEMINI_API_KEY=AIza-test-google
        ANTHROPIC_API_KEY='sk-ant-test'
        GITHUB_TOKEN=github_pat_test
        EMPTY=
        "#,
    );
    assert_eq!(found.len(), 4);
    assert!(found.iter().all(|obs| !obs.raw_value.is_empty()));
    assert!(found
        .iter()
        .any(|obs| obs.provider == CredentialProvider::OpenAi));
    assert!(found
        .iter()
        .any(|obs| obs.provider == CredentialProvider::Google));
    assert!(found
        .iter()
        .any(|obs| obs.provider == CredentialProvider::Anthropic));
    assert!(found
        .iter()
        .any(|obs| obs.provider == CredentialProvider::Github));
}

#[test]
fn http_detector_detects_github_authorization_without_raw_leak() {
    let obs = detect_http_credential(
        "api.github.com",
        "authorization",
        b"Bearer github_pat_secret",
    )
    .expect("github token should be detected");
    assert_eq!(obs.provider, CredentialProvider::Github);
    let event = obs.redacted_event("captured");
    assert!(is_broker_reference(&event.substitution_ref));
    assert!(!event.substitution_ref.contains("github_pat_secret"));
    assert!(!event.context_json.unwrap().contains("github_pat_secret"));
}

#[test]
fn http_body_detector_finds_github_token_exchange_and_redacts_body() {
    let body = br#"{"access_token":"github_pat_body_secret","token_type":"bearer"}"#;
    let found = detect_http_body_credentials(
        "api.github.com",
        "/login/oauth/access_token",
        "response",
        body,
    );

    assert_eq!(found.len(), 1);
    assert_eq!(found[0].provider, CredentialProvider::Github);
    assert_eq!(found[0].raw_value, "github_pat_body_secret");
    let redacted = redact_observed_credentials_in_bytes(body, &found);
    let redacted = String::from_utf8(redacted).unwrap();
    assert!(redacted.contains("credential:blake3:"));
    assert!(!redacted.contains("github_pat_body_secret"));
}

#[test]
fn http_body_detector_finds_google_oauth_json_response_without_token_prefix() {
    let body = br#"{"access_token":"ya29.live-access-token","refresh_token":"1//live-refresh-token","expires_in":3599}"#;
    let found = detect_http_body_credentials("oauth2.googleapis.com", "/token", "response", body);

    assert_eq!(found.len(), 2);
    assert!(found
        .iter()
        .all(|obs| obs.provider == CredentialProvider::Google));
    assert!(found
        .iter()
        .any(|obs| obs.source == "http.body.response.$.access_token"));
    assert!(found
        .iter()
        .any(|obs| obs.source == "http.body.response.$.refresh_token"));

    let redacted = String::from_utf8(redact_observed_credentials_in_bytes(body, &found)).unwrap();
    assert!(redacted.contains("credential:blake3:"));
    assert!(!redacted.contains("ya29.live-access-token"));
    assert!(!redacted.contains("1//live-refresh-token"));
}

#[test]
fn http_body_detector_finds_google_oauth_form_request() {
    let body = b"grant_type=authorization_code&code=4%2F0AfJohXsecret&client_id=public-client";
    let found = detect_http_body_credentials("oauth2.googleapis.com", "/token", "request", body);

    assert_eq!(found.len(), 1);
    assert_eq!(found[0].provider, CredentialProvider::Google);
    assert_eq!(found[0].raw_value, "4/0AfJohXsecret");
    assert_eq!(found[0].source, "http.body.request.form.code");

    let redacted = String::from_utf8(redact_observed_credentials_in_bytes(body, &found)).unwrap();
    assert!(redacted.contains("credential:blake3:"));
    assert!(!redacted.contains("4/0AfJohXsecret"));
}

#[test]
fn http_body_detector_finds_local_oauth_fixture_response() {
    let body = br#"{"access_token":"capsem_test_oauth_access_0123456789abcdef","refresh_token":"capsem_test_oauth_refresh_0123456789abcdef"}"#;
    let found = detect_http_body_credentials("127.0.0.1", "/oauth/token", "response", body);

    assert_eq!(found.len(), 2);
    assert!(found
        .iter()
        .all(|obs| obs.provider == CredentialProvider::Google));
    assert!(found
        .iter()
        .any(|obs| obs.source == "http.body.response.$.access_token"));
    assert!(found
        .iter()
        .any(|obs| obs.source == "http.body.response.$.refresh_token"));

    let redacted = String::from_utf8(redact_observed_credentials_in_bytes(body, &found)).unwrap();
    assert!(redacted.contains("credential:blake3:"));
    assert!(!redacted.contains("capsem_test_oauth_access_0123456789abcdef"));
    assert!(!redacted.contains("capsem_test_oauth_refresh_0123456789abcdef"));
}

#[test]
fn http_body_credential_candidate_is_limited_to_known_exchange_paths() {
    assert!(is_http_body_credential_candidate(
        "oauth2.googleapis.com",
        "/token"
    ));
    assert!(is_http_body_credential_candidate(
        "api.github.com",
        "/login/oauth/access_token"
    ));
    assert!(!is_http_body_credential_candidate(
        "daily-cloudcode-pa.googleapis.com",
        "/v1internal:streamGenerateContent"
    ));
    assert!(is_http_body_credential_candidate(
        "127.0.0.1",
        "/oauth/token"
    ));
    assert!(is_http_body_credential_candidate(
        "localhost",
        "/oauth/token"
    ));
    assert!(!is_http_body_credential_candidate("example.com", "/token"));
}

#[test]
fn substitution_is_domain_separated_by_provider() {
    let raw = "shared-token";
    let github = substitute_credential_value(CredentialProvider::Github, raw);
    let openai = substitute_credential_value(CredentialProvider::OpenAi, raw);
    assert_ne!(github, openai);
    assert!(is_broker_reference(&github));
    assert!(is_broker_reference(&openai));
}

#[test]
fn broker_stores_secret_without_writing_user_settings() {
    let _lock = TEST_ENV_LOCK.blocking_lock();
    let dir = tempfile::tempdir().unwrap();
    let capsem_home = dir.path().join("capsem-home");
    let test_store = dir.path().join("credential-store.json");
    let _guard = EnvGuard::install(&capsem_home, dir.path(), &test_store);

    let obs = CredentialObservation {
        provider: CredentialProvider::Github,
        raw_value: "github_pat_store_me".to_string(),
        source: "http.header.authorization".to_string(),
        event_type: Some("http.request".to_string()),
        confidence: 1.0,
        trace_id: Some("trace-test".to_string()),
        context_json: None,
    };

    let brokered = broker_observed_credential(&obs).unwrap();
    assert!(is_broker_reference(&brokered.credential_ref));
    assert_eq!(
        brokered.keychain_account,
        keychain_account(CredentialProvider::Github, &brokered.credential_ref)
    );

    assert!(
        !capsem_home.join("settings.toml").exists(),
        "credential broker must not create settings files for credential refs"
    );

    assert_eq!(
        resolve_broker_reference_for_provider(CredentialProvider::Github, &brokered.credential_ref)
            .unwrap()
            .as_deref(),
        Some("github_pat_store_me")
    );
    assert!(!brokered.credential_ref.contains("github_pat_store_me"));
}

#[test]
fn replay_availability_requires_resolvable_broker_secret() {
    let _lock = TEST_ENV_LOCK.blocking_lock();
    let dir = tempfile::tempdir().unwrap();
    let capsem_home = dir.path().join("capsem-home");
    let test_store = dir.path().join("credential-store.json");
    let _guard = EnvGuard::install(&capsem_home, dir.path(), &test_store);

    let missing = credential_reference("google", "not-stored");
    assert!(!broker_reference_replay_available(Some("google"), &missing));

    let brokered = broker_observed_credential(&CredentialObservation {
        provider: CredentialProvider::Google,
        raw_value: "ya29.refresh-token".to_string(),
        source: "http.body.response.$.refresh_token".to_string(),
        event_type: Some("http.response".to_string()),
        confidence: 1.0,
        trace_id: Some("trace-oauth".to_string()),
        context_json: None,
    })
    .unwrap();
    assert!(broker_reference_replay_available(
        Some("google"),
        &brokered.credential_ref
    ));
    assert!(broker_reference_replay_available(
        None,
        &brokered.credential_ref
    ));
    assert!(!broker_reference_replay_available(
        Some("openai"),
        &brokered.credential_ref
    ));
}
