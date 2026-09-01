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
