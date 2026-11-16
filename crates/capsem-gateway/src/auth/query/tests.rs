use super::*;
use axum::body::Body;

fn request(uri: &str) -> Request {
    let mut request = http::Request::get(uri)
        .header(header::CONNECTION, "keep-alive, Upgrade")
        .header(header::UPGRADE, "websocket")
        .header(header::SEC_WEBSOCKET_VERSION, "13")
        .header(header::SEC_WEBSOCKET_KEY, "dGhlIHNhbXBsZSBub25jZQ==")
        .body(Body::empty())
        .unwrap();
    let original = OriginalUri(request.uri().clone());
    request.extensions_mut().insert(original);
    request
}

#[test]
fn operator_query_is_removed_from_both_uris_and_becomes_a_sensitive_header() {
    let mut request = request("/events?left=a%2Bb&to%6ben=tok&right=1%202");
    canonicalize(&mut request, "tok").unwrap();
    assert_eq!(request.uri().to_string(), "/events?left=a%2Bb&right=1%202");
    assert_eq!(request.extensions().get::<OriginalUri>().unwrap().0, *request.uri());
    assert_eq!(request.headers()[header::AUTHORIZATION], "Bearer tok");
    assert!(request.headers()[header::AUTHORIZATION].is_sensitive());
}

#[test]
fn rejected_sources_are_stripped_without_overwriting_a_header_credential() {
    for query in [
        "token=tok&token=tok",
        "token=tok&to%6ben=tok",
        "Token=tok",
        "token=wrong",
    ] {
        let mut request = request(&format!("/events?{query}"));
        request
            .headers_mut()
            .insert(header::AUTHORIZATION, HeaderValue::from_static("Bearer original"));
        assert!(canonicalize(&mut request, "tok").is_err());
        assert_eq!(request.uri().to_string(), "/events");
        assert_eq!(request.extensions().get::<OriginalUri>().unwrap().0, *request.uri());
        assert_eq!(request.headers()[header::AUTHORIZATION], "Bearer original");
    }
}

#[test]
fn absent_credentials_preserve_query_bytes_and_invalid_paths_cannot_accept_tokens() {
    let mut plain = request("/events?left=a%2Bb&&right=1%202");
    canonicalize(&mut plain, "tok").unwrap();
    assert_eq!(plain.uri().to_string(), "/events?left=a%2Bb&&right=1%202");
    for path in [
        "/vms/list",
        "/vms/../stream",
        "/vms/box/stream/",
        "//vms/box/stream",
        "/vms/box%2Fother/stream",
    ] {
        let mut request = request(&format!("{path}?token=tok"));
        assert!(canonicalize(&mut request, "tok").is_err());
        assert_eq!(request.uri().query(), None);
    }
}
