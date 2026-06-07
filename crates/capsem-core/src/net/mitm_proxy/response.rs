pub(super) fn response_uses_gzip_content_encoding(headers: &http::HeaderMap) -> bool {
    headers
        .get(http::header::CONTENT_ENCODING)
        .and_then(|value| value.to_str().ok())
        .map(|value| {
            value
                .split(',')
                .any(|token| token.trim().eq_ignore_ascii_case("gzip"))
        })
        .unwrap_or(false)
}
