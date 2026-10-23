use super::*;

#[tokio::test]
async fn text_and_echo_preserve_exact_fixture_bytes() {
    let text = route(
        &Method::GET,
        "/content/text",
        Some("bytes=37"),
        &HeaderMap::new(),
        Bytes::new(),
    );
    assert_eq!(text.status(), StatusCode::OK);
    assert_eq!(text.into_body().collect().await.unwrap().to_bytes(), vec![b'x'; 37]);
    let mut headers = HeaderMap::new();
    headers.insert("content-type", "application/octet-stream".parse().unwrap());
    let bytes = Bytes::from_static(b"\xff\xe2\x82");
    let echo = route(&Method::POST, "/content/echo", None, &headers, bytes.clone());
    assert_eq!(echo.headers()["content-type"], "application/octet-stream");
    assert_eq!(echo.into_body().collect().await.unwrap().to_bytes(), bytes);
}

#[tokio::test]
async fn stream_produces_the_first_event_before_the_delayed_tail() {
    let stream = route(
        &Method::GET,
        "/content/stream",
        Some("bytes=20&delay_ms=100"),
        &HeaderMap::new(),
        Bytes::new(),
    );
    assert_eq!(stream.headers()["content-type"], "text/event-stream");
    let mut body = stream.into_body();
    assert_eq!(
        body.frame().await.unwrap().unwrap().into_data().unwrap(),
        "data: begin\n\n"
    );
    assert!(tokio::time::timeout(Duration::from_millis(10), body.frame())
        .await
        .is_err());
    assert_eq!(
        body.frame().await.unwrap().unwrap().into_data().unwrap(),
        format!("data: {}\n\ndata: end\n\n", "x".repeat(20))
    );
    assert!(body.frame().await.is_none());
}

#[test]
fn generated_fixture_controls_refuse_invalid_or_excessive_sizes() {
    for query in ["bytes=16777217", "bytes=bad", "bytes=1&bytes=2", "delay_ms=1001"] {
        let response = route(
            &Method::GET,
            "/content/text",
            Some(query),
            &HeaderMap::new(),
            Bytes::new(),
        );
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{query}");
    }
}

#[tokio::test]
async fn compressed_echo_retains_exact_gzip_bytes() {
    use std::io::Read;
    let body = Bytes::from_static(b"demo.person@example.test");
    let mut headers = HeaderMap::new();
    headers.insert("content-type", "text/plain".parse().unwrap());
    let result = route(&Method::POST, "/content/echo", Some("gzip=1"), &headers, body.clone());
    assert_eq!(result.headers()["content-encoding"], "gzip");
    let bytes = result.into_body().collect().await.unwrap().to_bytes();
    let mut decoded = Vec::new();
    flate2::read::GzDecoder::new(bytes.as_ref())
        .read_to_end(&mut decoded)
        .unwrap();
    assert_eq!(decoded, body);
}
