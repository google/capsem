use super::*;
use crate::credential_broker::CredentialProvider;

#[test]
fn header_formatter_sanitizes_and_emits_broker_observations() {
    let mut headers = hyper::HeaderMap::new();
    headers.insert(
        hyper::header::AUTHORIZATION,
        hyper::header::HeaderValue::from_static("Bearer sk-network-format-secret"),
    );

    let formatted = format_headers_for_domain("127.0.0.1", Some(ProviderKind::OpenAi), &headers);

    assert_eq!(formatted.observations.len(), 1);
    assert_eq!(formatted.observations[0].provider, CredentialProvider::OpenAi);
    assert_eq!(formatted.observations[0].source, "http.header.authorization");
    assert_eq!(formatted.observations[0].event_type.as_deref(), Some("http.request"));
    assert_eq!(
        formatted.credential_ref.as_deref(),
        Some(formatted.observations[0].credential_ref().as_str())
    );
    assert!(formatted.formatted.contains("authorization: hash:"));
    assert!(!formatted.formatted.contains("sk-network-format-secret"));
}

// -- Host header normalization --
//
// The Host header is guest-controlled. `parse_http_host_target` is the single
// place the plain-HTTP path derives its upstream from, so the host it returns
// must already be the normalized identity policy, dial, and telemetry share.

#[test]
fn parse_http_host_target_normalizes_case_and_trailing_dot() {
    let header = hyper::header::HeaderValue::from_static("Example.COM.");
    assert_eq!(
        parse_http_host_target(Some(&header)),
        Some(("example.com".to_string(), 80))
    );

    let with_port = hyper::header::HeaderValue::from_static("PASTEBIN.com.:8080");
    assert_eq!(
        parse_http_host_target(Some(&with_port)),
        Some(("pastebin.com".to_string(), 8080))
    );

    let ip = hyper::header::HeaderValue::from_static("127.0.0.1.:19222");
    assert_eq!(
        parse_http_host_target(Some(&ip)),
        Some(("127.0.0.1".to_string(), 19222))
    );
}

#[test]
fn parse_http_host_target_rejects_hosts_that_normalize_to_nothing() {
    for raw in [".", "..", ".:80", "..:8080", " . "] {
        let header = hyper::header::HeaderValue::from_str(raw).unwrap();
        assert_eq!(parse_http_host_target(Some(&header)), None, "{raw:?} names no host");
    }
}
