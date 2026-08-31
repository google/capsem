use super::*;

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
