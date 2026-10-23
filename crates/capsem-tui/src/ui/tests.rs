use super::*;

#[test]
fn guest_metadata_never_emits_terminal_controls_or_uses_them_as_visible_width() {
    assert_eq!(truncate("\u{1b}]52;c;payload\u{7}hello\r\n", 80), "]52;c;payloadhello");
    assert_eq!(truncate("\u{1b}\u{7}abc", 2), "ab...");
    assert_eq!(truncate("normal label", 80), "normal label");
}
