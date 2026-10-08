use std::net::SocketAddr;
use std::time::{Duration, Instant};

use super::*;

fn redirect() -> LoopbackRedirect {
    LoopbackRedirect::new("127.0.0.1:4123".parse().unwrap(), "/google/callback").unwrap()
}

fn attempt(now: Instant) -> OAuthAttempt {
    OAuthAttempt::new(
        redirect(),
        OAuthPolicy {
            lifetime: Duration::from_secs(30),
            max_callback_bytes: 1024,
        },
        now,
    )
    .unwrap()
}

fn callback(state: &str, tail: &str) -> String {
    format!("http://127.0.0.1:4123/google/callback?state={state}&{tail}")
}

#[test]
fn s256_matches_the_rfc7636_appendix_b_vector() {
    assert_eq!(
        s256("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
        "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
    );
}

#[test]
fn generated_material_is_distinct_unreserved_and_only_the_verifier_proves_the_challenge() {
    let now = Instant::now();
    let mut first = attempt(now);
    let mut second = attempt(now);
    let params = first.authorization(now).unwrap();
    let state = params.state.to_string();
    let challenge = params.code_challenge.to_string();
    assert_eq!(params.code_challenge_method, "S256");
    assert_eq!(params.redirect_uri, "http://127.0.0.1:4123/google/callback");
    let other = second.authorization(now).unwrap();
    for value in [&state, &challenge, other.state, other.code_challenge] {
        assert_eq!(value.len(), 43);
        assert!(value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'));
    }
    assert_ne!(state, other.state);
    assert_ne!(state, challenge);
    assert_ne!(challenge, other.code_challenge);
    let exchange = first
        .accept(&callback(&state, "code=opaque%2Bcode+value"), now)
        .unwrap();
    assert_eq!(exchange.code(), "opaque+code value");
    assert_eq!(s256(exchange.verifier()), challenge);
    assert_ne!(exchange.verifier(), state);
    assert_eq!(exchange.redirect_uri(), "http://127.0.0.1:4123/google/callback");
    assert_eq!(first.status(now), OAuthState::Consumed);
    assert!(matches!(
        first.accept(&callback(&state, "code=replay"), now),
        Err(OAuthError::Inactive(OAuthState::Consumed))
    ));
}

#[test]
fn wrong_state_and_ambiguous_or_malformed_parameters_do_not_consume_the_transaction() {
    let now = Instant::now();
    let mut owner = attempt(now);
    let state = owner.authorization(now).unwrap().state.to_string();
    assert!(matches!(
        owner.accept(&callback("wrong-state", "code=valid"), now),
        Err(OAuthError::InvalidState)
    ));
    for tail in [
        "code=a&code=b",
        "code=a&%73tate=duplicate",
        "code=a&error=denied",
        "error=a&error=b",
        "code=",
        "error=",
        "code=%00",
        "code=%FF",
        "code=%ZZ",
        "scope=read",
    ] {
        assert!(
            matches!(
                owner.accept(&callback(&state, tail), now),
                Err(OAuthError::InvalidCallback)
            ),
            "{tail}"
        );
        assert_eq!(owner.status(now), OAuthState::Pending);
    }
    assert!(matches!(
        owner.accept("http://127.0.0.1:4123/google/callback?code=a", now),
        Err(OAuthError::InvalidState)
    ));
    owner
        .accept(&callback(&state, "code=valid&scope=read&authuser=0"), now)
        .unwrap();
}

#[test]
fn callback_origin_port_path_and_fragment_must_match_the_bound_literal_target() {
    let now = Instant::now();
    let mut owner = attempt(now);
    let state = owner.authorization(now).unwrap().state.to_string();
    for base in [
        "https://127.0.0.1:4123/google/callback",
        "http://localhost:4123/google/callback",
        "http://127.0.0.1:4124/google/callback",
        "http://127.0.0.2:4123/google/callback",
        "http://127.0.0.1:4123/other",
        "http://127.0.0.1:4123/google/../google/callback",
        "http://127.0.0.1:4123/google/%63allback",
        "http://user@127.0.0.1:4123/google/callback",
    ] {
        assert!(matches!(
            owner.accept(&format!("{base}?state={state}&code=a"), now),
            Err(OAuthError::InvalidCallback)
        ));
    }
    assert!(matches!(
        owner.accept(&format!("{}#fragment", callback(&state, "code=a")), now),
        Err(OAuthError::InvalidCallback)
    ));
    owner.accept(&callback(&state, "code=valid"), now).unwrap();
}

#[test]
fn denial_cancellation_and_exact_expiry_never_yield_exchange_material() {
    let now = Instant::now();
    let mut denied = attempt(now);
    let state = denied.authorization(now).unwrap().state.to_string();
    assert!(matches!(
        denied.accept(&callback(&state, "error=access_denied&error_description=private"), now),
        Err(OAuthError::Denied)
    ));
    assert_eq!(denied.status(now), OAuthState::Denied);
    assert!(matches!(
        denied.accept(&callback(&state, "code=late"), now),
        Err(OAuthError::Inactive(OAuthState::Denied))
    ));
    let mut cancelled = attempt(now);
    let state = cancelled.authorization(now).unwrap().state.to_string();
    cancelled.cancel();
    cancelled.cancel();
    assert_eq!(cancelled.status(now), OAuthState::Cancelled);
    assert!(matches!(
        cancelled.accept(&callback(&state, "code=late"), now),
        Err(OAuthError::Inactive(OAuthState::Cancelled))
    ));
    let mut expired = attempt(now);
    let state = expired.authorization(now).unwrap().state.to_string();
    assert!(matches!(
        expired.accept(&callback(&state, "code=late"), now + Duration::from_secs(30)),
        Err(OAuthError::Inactive(OAuthState::Expired))
    ));
    assert!(matches!(
        expired.authorization(now),
        Err(OAuthError::Inactive(OAuthState::Expired))
    ));
}

#[test]
fn provider_failure_is_terminal_and_distinct_from_denied_consent() {
    let now = Instant::now();
    let mut owner = attempt(now);
    let state = owner.authorization(now).unwrap().state.to_string();
    assert!(matches!(
        owner.accept(&callback(&state, "error=server_error&error_description=private"), now),
        Err(OAuthError::ProviderRejected)
    ));
    assert_eq!(owner.status(now), OAuthState::Failed);
    assert!(matches!(
        owner.accept(&callback(&state, "code=late"), now),
        Err(OAuthError::Inactive(OAuthState::Failed))
    ));
}

#[test]
fn loopback_redirect_and_policy_validation_have_no_implicit_defaults() {
    for target in ["192.0.2.1:4123", "127.0.0.1:0", "[2001:db8::1]:4123"] {
        let address: SocketAddr = target.parse().unwrap();
        assert!(matches!(
            LoopbackRedirect::new(address, "/callback"),
            Err(OAuthError::InvalidRedirect)
        ));
    }
    for path in [
        "",
        "callback",
        "//host/callback",
        "/a/../callback",
        "/%63allback",
        "/callback?x=y",
        "/callback#x",
        "/call\\back",
        "/call\nback",
    ] {
        assert!(matches!(
            LoopbackRedirect::new("127.0.0.1:4123".parse().unwrap(), path),
            Err(OAuthError::InvalidRedirect)
        ));
    }
    assert_eq!(
        LoopbackRedirect::new("[::1]:4123".parse().unwrap(), "/callback")
            .unwrap()
            .uri(),
        "http://[::1]:4123/callback"
    );
    let now = Instant::now();
    for policy in [
        OAuthPolicy {
            lifetime: Duration::ZERO,
            max_callback_bytes: 1024,
        },
        OAuthPolicy {
            lifetime: Duration::MAX,
            max_callback_bytes: 1024,
        },
        OAuthPolicy {
            lifetime: Duration::from_secs(30),
            max_callback_bytes: 0,
        },
    ] {
        assert!(matches!(
            OAuthAttempt::new(redirect(), policy, now),
            Err(OAuthError::InvalidPolicy)
        ));
    }
    let mut bounded = attempt(now);
    let state = bounded.authorization(now).unwrap().state.to_string();
    assert!(matches!(
        bounded.accept(&callback(&state, &format!("code={}", "x".repeat(1024))), now),
        Err(OAuthError::InvalidCallback)
    ));
    bounded.accept(&callback(&state, "code=valid"), now).unwrap();
}

#[test]
fn debug_and_errors_redact_state_verifier_code_and_callback_details() {
    let now = Instant::now();
    let mut owner = attempt(now);
    let params = owner.authorization(now).unwrap();
    let state = params.state.to_string();
    assert!(!format!("{params:?}").contains(&state));
    let pending_verifier = owner.pending.as_ref().unwrap().verifier.expose().to_string();
    let pending_debug = format!("{owner:?}");
    assert!(!pending_debug.contains(&state));
    assert!(!pending_debug.contains(&pending_verifier));
    let exchange = owner.accept(&callback(&state, "code=private-code"), now).unwrap();
    for display in [
        format!("{owner:?}"),
        format!("{exchange:?}"),
        OAuthError::InvalidCallback.to_string(),
    ] {
        for secret in [&state, exchange.verifier(), exchange.code()] {
            assert!(!display.contains(secret));
        }
    }
    owner.cancel();
    assert_eq!(owner.status(now), OAuthState::Consumed);
}
