use super::*;

#[test]
fn meta_line_uses_the_host_wire_format() {
    assert_eq!(encode_meta_line("claude"), b"\0CAPSEM_META:claude\n");
}

#[test]
fn sanitizes_process_names_before_encoding() {
    assert_eq!(sanitize_process_name("clean"), "clean");
    assert_eq!(sanitize_process_name("has space"), "has_space");
    assert_eq!(sanitize_process_name("has\nnewline"), "has_newline");
    assert_eq!(sanitize_process_name("has\rcarriage"), "has_carriage");
    assert_eq!(sanitize_process_name("has\0nul"), "has_nul");
    assert_eq!(sanitize_process_name("has\ttab"), "has_tab");
    assert_eq!(
        sanitize_process_name("claude/code-v4.0"),
        "claude/code-v4.0"
    );
}

#[test]
fn truncates_multibyte_names_by_character() {
    let name = format!("{}é", "a".repeat(127));
    let result = sanitize_process_name(&name);

    assert_eq!(result.chars().count(), MAX_PROCESS_NAME_CHARS);
    assert!(result.ends_with('é'));
}

#[test]
fn encoded_meta_line_cannot_be_injected() {
    let meta = encode_meta_line("evil\nCAPSEM_META:spoof");

    assert_eq!(meta.iter().filter(|&&byte| byte == b'\n').count(), 1);
    assert_eq!(meta.iter().filter(|&&byte| byte == 0).count(), 1);
}
