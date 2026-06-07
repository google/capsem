//! Tests for `cors` (extracted from inline `mod tests`).

use super::is_allowed_origin;
use http::HeaderValue;

fn allowed(origin: &str) -> bool {
    is_allowed_origin(&HeaderValue::from_str(origin).expect("valid header bytes"))
}

// --- Positive cases: same-machine origins we trust ---

#[test]
fn allows_http_localhost_with_port() {
    assert!(allowed("http://localhost:4321"));
}

#[test]
fn allows_https_localhost_with_port() {
    assert!(allowed("https://localhost:4321"));
}

#[test]
fn allows_http_localhost_no_port() {
    assert!(allowed("http://localhost"));
}

#[test]
fn allows_http_127_0_0_1_with_port() {
    assert!(allowed("http://127.0.0.1:19222"));
}

#[test]
fn allows_https_127_0_0_1() {
    assert!(allowed("https://127.0.0.1:19222"));
}

#[test]
fn allows_http_ipv6_loopback_bracketed() {
    assert!(allowed("http://[::1]:19222"));
}

#[test]
fn allows_tauri_localhost() {
    assert!(allowed("tauri://localhost"));
}

#[test]
fn allows_localhost_case_insensitive() {
    assert!(allowed("http://LocalHost:4321"));
}

// --- Negative cases: AB-001 ---

#[test]
fn rejects_localhost_suffix_attack() {
    // Bug AB-001: prefix-only match would approve this attacker-controlled host.
    assert!(!allowed("http://localhostevil.com"));
}

#[test]
fn rejects_localhost_dot_suffix_attack() {
    assert!(!allowed("http://localhost.evil.example"));
}

#[test]
fn rejects_127_0_0_1_dot_suffix_attack() {
    assert!(!allowed("http://127.0.0.1.evil.example"));
}

#[test]
fn rejects_tauri_non_localhost_host() {
    // The `tauri://` scheme is meaningful only with `localhost`; any other
    // host under that scheme is attacker-defined surface.
    assert!(!allowed("tauri://evil.example"));
    assert!(!allowed("tauri://localhost.evil.example"));
}

#[test]
fn rejects_external_origin() {
    assert!(!allowed("https://evil.example.com"));
}

#[test]
fn rejects_unknown_scheme() {
    assert!(!allowed("ftp://localhost"));
    assert!(!allowed("file://localhost"));
    assert!(!allowed("javascript://localhost"));
}

#[test]
fn rejects_origin_with_path() {
    // Real Origin headers never carry a path beyond `/`. Anything richer
    // implies a malformed/tampered value.
    assert!(!allowed("http://localhost:4321/extra"));
}

#[test]
fn rejects_origin_with_userinfo() {
    // `http://user@host` has a different effective host in some parsers.
    assert!(!allowed("http://attacker@localhost:4321"));
}

#[test]
fn rejects_origin_with_query() {
    assert!(!allowed("http://localhost:4321?foo=bar"));
}

#[test]
fn rejects_origin_with_fragment() {
    assert!(!allowed("http://localhost:4321#frag"));
}

#[test]
fn rejects_empty_string() {
    assert!(!allowed(""));
}

#[test]
fn rejects_garbage() {
    assert!(!allowed("not a uri at all"));
}

#[test]
fn rejects_non_ascii_header() {
    // Origin must be ASCII per RFC. Build a non-UTF8/non-ASCII byte sequence
    // and confirm we reject it without panicking.
    let bytes = HeaderValue::from_bytes(b"http://l\xff.example").unwrap();
    assert!(!is_allowed_origin(&bytes));
}

#[test]
fn rejects_scheme_only() {
    assert!(!allowed("http://"));
}
