use super::*;

async fn routed(
    method: Method,
    path: &str,
    query: Option<&str>,
    headers: HeaderMap,
    body: Value,
) -> (StatusCode, HeaderMap, Bytes) {
    let response = route(
        &method,
        path,
        query,
        &headers,
        Bytes::from(serde_json::to_vec(&body).unwrap()),
        false,
    )
    .await;
    let status = response.status();
    let headers = response.headers().clone();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, headers, body)
}

async fn routed_json(method: Method, path: &str, body: Value) -> Value {
    let (status, _, bytes) = routed(method, path, None, HeaderMap::new(), body).await;
    assert_eq!(status, StatusCode::OK, "unexpected response for {path}");
    serde_json::from_slice(&bytes).unwrap_or_else(|error| {
        panic!("{path} returned non-JSON {bytes:?}: {error}");
    })
}

#[test]
fn parent_watch_is_optional_but_parsed_when_a_launcher_owns_the_server() {
    let standalone = Args::try_parse_from(["capsem-mock-server"]).expect("standalone mock server arguments");
    assert_eq!(standalone.parent_pid, None);

    let guarded =
        Args::try_parse_from(["capsem-mock-server", "--parent-pid", "4242"]).expect("guarded mock server arguments");
    assert_eq!(guarded.parent_pid, Some(4242));
}

#[test]
fn deterministic_bytes_are_cached_and_correct() {
    let first = deterministic_bytes("10mb").expect("10mb fixture");
    let second = deterministic_bytes("10mb").expect("10mb fixture");
    assert_eq!(first.len(), 10 * 1024 * 1024);
    assert_eq!(first, second);
    assert_eq!(&first[..26], b"abcdefghijklmnopqrstuvwxyz");
}

#[test]
fn dns_fixture_answers_known_names_and_rejects_unknown() {
    let query = test_dns_query("fixture.capsem.test", 0xCAFE);
    let response = dns_response(&query).expect("dns response");
    assert_eq!(&response[..2], b"\xCA\xFE");
    assert_eq!(response[3] & 0x0F, 0);
    assert_eq!(&response[response.len() - 4..], &[127, 0, 0, 1]);

    let query = test_dns_query("unknown.capsem.invalid", 0xBEEF);
    let response = dns_response(&query).expect("dns response");
    assert_eq!(&response[..2], b"\xBE\xEF");
    assert_eq!(response[3] & 0x0F, 3);
}

#[test]
fn websocket_accept_matches_rfc_fixture() {
    assert_eq!(
        websocket_accept("dGhlIHNhbXBsZSBub25jZQ=="),
        "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
    );
}

fn test_dns_query(name: &str, id: u16) -> Vec<u8> {
    let mut query = Vec::new();
    query.extend_from_slice(&id.to_be_bytes());
    query.extend_from_slice(&[0x01, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0]);
    for part in name.split('.') {
        query.push(u8::try_from(part.len()).expect("label fits"));
        query.extend_from_slice(part.as_bytes());
    }
    query.extend_from_slice(&[0, 0, 1, 0, 1]);
    query
}

// ── Request-parsing helpers ────────────────────────────────────────
//
// This binary is the single shared fixture behind benchmarks, doctor, protocol
// replay, gateway integration and Ironbank. Several of its parsers fail into a
// default rather than an error, which is fine for a fixture but means a test
// built on a malformed request can pass for the wrong reason. These pin where
// the silent defaults are, so a suite that relies on one is relying on stated
// behaviour rather than an accident.

#[test]
fn malformed_request_bodies_parse_to_an_empty_object_not_a_panic() {
    for body in [
        &b""[..],
        &b"not json"[..],
        &b"{\"unterminated\": "[..],
        &b"\xff\xfe\xfd"[..], // invalid UTF-8
        &b"[1,2,3]"[..],      // valid JSON, wrong shape
    ] {
        let parsed = parse_json(body);
        assert!(
            parsed.is_object() || parsed.is_array(),
            "unexpected parse of {body:?}: {parsed}"
        );
    }
    // The silent-default case worth stating outright: junk becomes {}, so a
    // handler reading a field sees "absent", never "malformed".
    assert_eq!(parse_json(b"not json"), json!({}));
    assert_eq!(parse_json(b""), json!({}));
}

#[test]
fn well_formed_bodies_are_returned_intact() {
    assert_eq!(
        parse_json(br#"{"model":"gpt-4","n":2}"#),
        json!({"model": "gpt-4", "n": 2})
    );
}

#[test]
fn google_model_is_read_from_the_path_and_falls_back_when_absent() {
    assert_eq!(
        google_model_from_path("/v1/models/gemini-2.0-pro:generateContent"),
        "gemini-2.0-pro"
    );
    assert_eq!(google_model_from_path("/v1/models/gemini-2.0-pro"), "gemini-2.0-pro");

    // Every shape that cannot name a model resolves to the same default. A
    // routing test asserting on the model must therefore assert on a path that
    // actually carries one, or it proves nothing.
    for path in [
        "/v1/generateContent",  // no /models/ segment
        "/v1/models/",          // empty model
        "/v1/models/:generate", // empty before the colon
        "",
    ] {
        assert_eq!(
            google_model_from_path(path),
            "gemini-3.5-flash",
            "{path:?} should fall back"
        );
    }
}

#[test]
fn google_model_takes_the_first_segment_after_models() {
    // A second /models/ later in the path must not win.
    assert_eq!(google_model_from_path("/v1/models/first:x/models/second"), "first");
}

#[test]
fn hex32_scan_requires_a_full_thirty_two_character_run() {
    let thirty_two = "a".repeat(32);
    assert_eq!(find_hex32(&thirty_two), Some(thirty_two.clone()));

    assert_eq!(find_hex32(&"a".repeat(31)), None, "31 hex chars is not a token");
    assert_eq!(find_hex32(""), None);
    assert_eq!(find_hex32("nothing hexadecimal in here at all!!"), None);
}

#[test]
fn hex32_scan_finds_a_token_embedded_in_surrounding_text() {
    let token = "0123456789abcdef0123456789ABCDEF";
    let raw = format!(r#"{{"session":"{token}","note":"zz"}}"#);

    assert_eq!(find_hex32(&raw).as_deref(), Some(token));
}

#[test]
fn hex32_scan_handles_non_ascii_without_panicking() {
    // The window scan walks raw bytes, so multi-byte characters must not be
    // able to slice a token out of the middle of a code point.
    assert_eq!(find_hex32("héllo ünïcode ✓ ✗ ★"), None);

    let token = "abcdef0123456789abcdef0123456789";
    assert_eq!(find_hex32(&format!("é{token}é")).as_deref(), Some(token));
}

#[test]
fn root_txt_path_is_extracted_and_stops_at_the_first_disallowed_character() {
    assert_eq!(
        find_root_txt_path(r#"{"cmd":"cat /root/out.txt"}"#).as_deref(),
        Some("/root/out.txt")
    );
    assert_eq!(
        find_root_txt_path("/root/nested/dir/file.txt and more").as_deref(),
        Some("/root/nested/dir/file.txt")
    );
}

#[test]
fn root_txt_path_ignores_candidates_without_a_txt_suffix() {
    assert_eq!(find_root_txt_path("/root/binary.bin"), None);
    assert_eq!(find_root_txt_path("no path here"), None);
    assert_eq!(find_root_txt_path(""), None);
}

#[test]
fn root_txt_path_takes_the_last_candidate_when_several_appear() {
    // `.last()` is load-bearing: a payload that mentions an earlier path in
    // prose must not beat the one the command actually writes.
    assert_eq!(
        find_root_txt_path("first /root/a.txt then /root/b.txt").as_deref(),
        Some("/root/b.txt")
    );
}

#[test]
fn header_lookup_is_case_insensitive_and_rejects_non_utf8_values() {
    let mut headers = HeaderMap::new();
    headers.insert("content-type", "application/json".parse().unwrap());
    headers.insert(
        "x-binary",
        hyper::header::HeaderValue::from_bytes(&[0xff, 0xfe]).unwrap(),
    );

    assert_eq!(request_header(&headers, "Content-Type"), Some("application/json"));
    assert_eq!(request_header(&headers, "content-type"), Some("application/json"));
    assert_eq!(request_header(&headers, "absent"), None);
    assert_eq!(
        request_header(&headers, "x-binary"),
        None,
        "a non-UTF-8 header value reads as absent rather than panicking"
    );
}

#[tokio::test]
async fn static_get_routes_return_their_contract_markers() {
    for (path, marker) in [
        ("/tiny", "capsem-mock-server:tiny"),
        ("/html/about", "Capsem mock server about page"),
        ("/html/large", "capsem-large-html"),
        ("/sse/model", "event: model.tool_call"),
        ("/model/response", "mock-model-response"),
        ("/oauth/authorize", "synthetic_oauth_authorization_fixture"),
        ("/credential/response", "synthetic_credential_fixture"),
        ("/api/tags", "gemma4:latest"),
        ("/deny-target", "deny-target"),
        ("/chunked", "chunk-3"),
    ] {
        let (status, _, body) = routed(Method::GET, path, None, HeaderMap::new(), json!({})).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert!(String::from_utf8_lossy(&body).contains(marker), "{path}: {body:?}");
    }

    let (status, _, body) = routed(Method::GET, "/absent", None, HeaderMap::new(), json!({})).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(body.is_empty());
}

#[tokio::test]
async fn generated_byte_routes_report_sizes_and_encoding() {
    let (status, headers, body) = routed(Method::GET, "/bytes/128", None, HeaderMap::new(), json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.len(), 128);
    assert_eq!(&body[..26], b"abcdefghijklmnopqrstuvwxyz");
    assert_eq!(headers[CONTENT_LENGTH], "128");

    let (status, headers, body) = routed(Method::GET, "/gzip/10kb", None, HeaderMap::new(), json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[CONTENT_ENCODING], "gzip");
    assert!(!body.is_empty());

    let (status, _, _) = routed(Method::GET, "/bytes/not-a-size", None, HeaderMap::new(), json!({})).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn provider_json_routes_preserve_requested_models_and_shapes() {
    let openai = routed_json(
        Method::POST,
        "/v1/chat/completions",
        json!({"model": "gpt-fixture", "messages": []}),
    )
    .await;
    assert_eq!(openai["model"], "gpt-fixture");
    assert_eq!(openai["choices"][0]["finish_reason"], "stop");

    let anthropic = routed_json(
        Method::POST,
        "/v1/messages",
        json!({"model": "claude-fixture", "messages": []}),
    )
    .await;
    assert_eq!(anthropic["model"], "claude-fixture");

    let ollama = routed_json(Method::POST, "/api/chat", json!({"model": "local-fixture"})).await;
    assert_eq!(ollama["model"], "local-fixture");

    let gemini = routed_json(
        Method::POST,
        "/v1/models/gemini-fixture:generateContent",
        json!({"contents": []}),
    )
    .await;
    assert_eq!(gemini["modelVersion"], "gemini-fixture");
}

#[tokio::test]
async fn provider_stream_routes_emit_protocol_terminators() {
    for (path, payload, marker) in [
        (
            "/v1/chat/completions",
            json!({"model": "gpt-fixture", "stream": true}),
            "data: [DONE]",
        ),
        (
            "/v1/responses",
            json!({"model": "gpt-fixture", "stream": true}),
            "response.completed",
        ),
        (
            "/v1/messages",
            json!({"model": "claude-fixture", "stream": true}),
            "message_stop",
        ),
        (
            "/v1/models/gemini-fixture:streamGenerateContent",
            json!({"contents": []}),
            "candidates",
        ),
    ] {
        let (status, headers, bytes) = routed(Method::POST, path, None, HeaderMap::new(), payload).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert!(String::from_utf8_lossy(&bytes).contains(marker), "{path}: {bytes:?}");
        assert!(headers[CONTENT_TYPE].to_str().unwrap().contains("event-stream"));
    }
}

#[tokio::test]
async fn auxiliary_post_routes_cover_openai_google_ollama_and_oauth_shapes() {
    for (path, marker) in [
        ("/v1/embeddings", "embedding"),
        ("/v1/images/generations", "b64_json"),
        ("/model/shape", "chatcmpl_shape_fixture"),
        ("/model/no-tool-call", "chatcmpl_no_tool_fixture"),
        ("/api/show", "modelfile"),
        ("/oauth/token", "access_token"),
        ("/api/client/features", "features"),
        ("/v1internal:fetchUserInfo", "userSettings"),
        ("/v1internal:fetchAvailableModels", "models"),
    ] {
        let value = routed_json(Method::POST, path, json!({"model": "fixture"})).await;
        assert!(
            value.get(marker).is_some() || value.to_string().contains(marker),
            "{path}: {value}"
        );
    }

    let (status, _, body) = routed(Method::POST, "/log", None, HeaderMap::new(), json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.is_empty());
}

#[tokio::test]
async fn echo_route_reports_sensitive_header_shapes_without_echoing_values() {
    let mut headers = HeaderMap::new();
    headers.insert("authorization", "Bearer credential:blake3:abc".parse().unwrap());
    headers.insert("cookie", "session=secret".parse().unwrap());
    headers.insert("x-api-key", "secret".parse().unwrap());
    headers.insert(CONTENT_TYPE, "application/json".parse().unwrap());
    let value = {
        let (status, _, bytes) = routed(
            Method::POST,
            "/echo",
            Some("access_token=credential%3Ablake3%3Adef"),
            headers,
            json!({"hello": "world"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        serde_json::from_slice::<Value>(&bytes).unwrap()
    };
    assert_eq!(value["has_authorization"], true);
    assert_eq!(value["authorization_is_broker_ref"], true);
    assert_eq!(value["query_has_access_token"], true);
    assert_eq!(value["has_cookie"], true);
    assert_eq!(value["has_x_api_key"], true);
    assert!(!value.to_string().contains("session=secret"));
}

#[tokio::test]
async fn mcp_route_handles_initialize_list_call_and_unknown_methods() {
    for (method, expected) in [
        ("initialize", "protocolVersion"),
        ("tools/list", "tools"),
        ("tools/call", "content"),
        ("resources/list", "resources"),
        ("resources/read", "contents"),
        ("prompts/list", "error"),
        ("unknown/method", "error"),
    ] {
        let payload = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": {"name": "echo", "arguments": {"text": "hello"}}
        });
        let value = routed_json(Method::POST, "/mcp", payload).await;
        assert!(
            value.get(expected).is_some() || value.get("result").and_then(|v| v.get(expected)).is_some(),
            "{method}: {value}"
        );
    }
}
