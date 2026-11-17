//! Pure bounded-field and Unicode truncation invariants.
use super::*;

#[test]
fn cap_field_none_returns_none() {
    assert!(cap_field(&None).is_none());
}

#[test]
fn cap_field_short_string_unchanged() {
    let s = Some("hello world".to_string());
    assert_eq!(cap_field(&s).as_deref(), Some("hello world"));
}

#[test]
fn cap_field_exact_limit_unchanged() {
    let s = Some("x".repeat(MAX_FIELD_BYTES));
    let result = cap_field(&s).unwrap();
    assert_eq!(result.len(), MAX_FIELD_BYTES);
}

#[test]
fn cap_field_over_limit_truncated() {
    let s = Some("a".repeat(MAX_FIELD_BYTES + 100));
    let result = cap_field(&s).unwrap();
    assert_eq!(result.len(), MAX_FIELD_BYTES);
}

#[test]
fn cap_field_utf8_boundary_safe() {
    // Multi-byte UTF-8: each char is 4 bytes
    let emoji = "\u{1F600}"; // 4-byte emoji
    assert_eq!(emoji.len(), 4);
    // Fill up to just past the limit with 4-byte chars
    let count = MAX_FIELD_BYTES / 4 + 1; // slightly over
    let s = Some(emoji.repeat(count));
    let result = cap_field(&s).unwrap();
    assert!(result.len() <= MAX_FIELD_BYTES);
    // Truncated at a char boundary -- must be valid UTF-8
    assert!(result.is_char_boundary(result.len()));
    // Length should be a multiple of 4 (each emoji is 4 bytes)
    assert_eq!(result.len() % 4, 0);
}

#[test]
fn cap_field_two_byte_utf8_boundary() {
    // 2-byte char: e.g. 'a' with accent
    let ch = "\u{00E9}"; // e-acute, 2 bytes
    assert_eq!(ch.len(), 2);
    let count = MAX_FIELD_BYTES / 2 + 1;
    let s = Some(ch.repeat(count));
    let result = cap_field(&s).unwrap();
    assert!(result.len() <= MAX_FIELD_BYTES);
    assert_eq!(result.len() % 2, 0);
}

#[test]
fn cap_field_three_byte_utf8_boundary() {
    // 3-byte char: CJK character
    let ch = "\u{4E16}"; // Chinese char, 3 bytes
    assert_eq!(ch.len(), 3);
    let count = MAX_FIELD_BYTES / 3 + 1;
    let s = Some(ch.repeat(count));
    let result = cap_field(&s).unwrap();
    assert!(result.len() <= MAX_FIELD_BYTES);
    assert_eq!(result.len() % 3, 0);
}

#[test]
fn cap_field_empty_string_unchanged() {
    let s = Some(String::new());
    assert_eq!(cap_field(&s).as_deref(), Some(""));
}

#[test]
fn cap_field_mixed_ascii_and_multibyte() {
    // Fill most of the buffer with ASCII, end with a 4-byte char that straddles the limit
    let mut s = "x".repeat(MAX_FIELD_BYTES - 1);
    s.push('\u{1F600}'); // 4 bytes, total = MAX_FIELD_BYTES + 3
    let result = cap_field(&Some(s)).unwrap();
    assert!(result.len() <= MAX_FIELD_BYTES);
    // Should have truncated to MAX_FIELD_BYTES - 1 (dropping the emoji)
    assert_eq!(result.len(), MAX_FIELD_BYTES - 1);
    assert!(result.chars().all(|c| c == 'x'));
}

#[test]
fn cap_bytes_max_zero_returns_empty_string() {
    let s = Some("anything".to_string());
    assert_eq!(cap_bytes(&s, 0).as_deref(), Some(""));
}

#[test]
fn cap_bytes_truncates_at_char_boundary_below_multibyte_char() {
    // "héllo": h(1) + é(2) + l(1) + l(1) + o(1) = 6 bytes. A max of 2 lands
    // mid-way through the 2-byte 'é', so the boundary search must back off
    // to the end of 'h' (byte 1), not split the char.
    let s = Some("héllo".to_string());
    assert_eq!(cap_bytes(&s, 2).as_deref(), Some("h"));
}

#[test]
fn cap_preview_caps_to_exactly_preview_bytes() {
    let s = Some("x".repeat(4 * 1024));
    let result = cap_preview(&s).unwrap();
    assert_eq!(result.len(), PREVIEW_BYTES);
    assert_eq!(PREVIEW_BYTES, 2048);
}
