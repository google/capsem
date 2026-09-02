//! Tests for `truncate_path` character-boundary handling.

use super::*;

// -----------------------------------------------------------------------
// AB-007: truncate_path -- char-boundary safe
// -----------------------------------------------------------------------

#[test]
fn truncate_path_ascii_under_max_returns_as_is() {
    assert_eq!(truncate_path("/a/b/c", 33), "/a/b/c");
}

#[test]
fn truncate_path_ascii_over_max_keeps_last_chars_with_ellipsis() {
    let path = "a".repeat(50);
    let out = truncate_path(&path, 33);
    assert_eq!(out.chars().count(), 33);
    assert!(out.starts_with("..."));
    assert_eq!(&out[3..], &"a".repeat(30));
}

#[test]
fn truncate_path_unicode_under_max_chars_is_kept_even_if_byte_len_exceeds() {
    // 10 CJK chars = 30 bytes; max 33 chars; should pass through unchanged.
    let path = "日".repeat(10);
    assert_eq!(truncate_path(&path, 33), path);
}

#[test]
fn truncate_path_unicode_does_not_panic_at_codepoint_boundary() {
    // AB-007 regression: with the legacy byte-slice implementation this
    // input panicked with "byte index N is not a char boundary" because
    // the suffix started in the middle of a multibyte character.
    //
    // 40 CJK (`日`, 3 bytes each) + 1 ASCII = 41 chars, 121 bytes.
    // max = 33. Legacy code computed slice start =
    // `path.len() - (max - 3) = 121 - 30 = 91`, which lands inside the
    // 31st `日` (bytes 90-92).
    let path = format!("{}a", "日".repeat(40));
    let out = truncate_path(&path, 33);
    assert!(out.starts_with("..."));
    assert_eq!(out.chars().count(), 33);
    let suffix: String = out.chars().skip(3).collect();
    assert_eq!(suffix, format!("{}a", "日".repeat(29)));
}

#[test]
fn truncate_path_unicode_over_max_uses_char_count_not_byte_count() {
    // 40 CJK chars = 120 bytes; max 33 chars; want last 30 chars + "...".
    let path = "日".repeat(40);
    let out = truncate_path(&path, 33);
    assert_eq!(out.chars().count(), 33);
    assert!(out.starts_with("..."));
    let suffix: String = out.chars().skip(3).collect();
    assert_eq!(suffix, "日".repeat(30));
}

#[test]
fn truncate_path_empty_string_returns_empty() {
    assert_eq!(truncate_path("", 33), "");
}

#[test]
fn truncate_path_max_three_returns_last_three_chars_no_ellipsis() {
    // With max == 3 there is no room for both an ellipsis and content;
    // returning the last `max` chars (no ellipsis) is more useful than
    // returning just "..." -- and importantly does not panic.
    let path = "abcdefghij";
    assert_eq!(truncate_path(path, 3), "hij");
}

#[test]
fn truncate_path_max_zero_does_not_panic() {
    // Defensive: ill-typed callers must not bring down snapshot rendering.
    let _ = truncate_path("abcdef", 0);
    let _ = truncate_path("日本語", 0);
}
