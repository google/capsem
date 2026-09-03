//! `tail_lines` / `grep_lines` log filtering helpers.

use super::*;

// -----------------------------------------------------------------------
// tail_lines / grep_lines
// -----------------------------------------------------------------------

#[test]
fn tail_lines_basic() {
    let text = "line 1\nline 2\nline 3\nline 4\nline 5";
    assert_eq!(tail_lines(text, 2), "line 4\nline 5");
}

#[test]
fn tail_lines_more_than_available() {
    let text = "line 1\nline 2";
    assert_eq!(tail_lines(text, 10), text);
}

#[test]
fn tail_lines_exact() {
    let text = "line 1\nline 2\nline 3";
    assert_eq!(tail_lines(text, 3), text);
}

#[test]
fn tail_lines_empty() {
    assert_eq!(tail_lines("", 5), "");
}

#[test]
fn tail_log_fields_applies_to_all() {
    let mut val = json!({
        "logs": "a\nb\nc\nd\ne",
        "serial_logs": "1\n2\n3\n4\n5",
        "process_logs": "x\ny\nz",
    });
    tail_log_fields(&mut val, 2);
    assert_eq!(val["logs"], "d\ne");
    assert_eq!(val["serial_logs"], "4\n5");
    assert_eq!(val["process_logs"], "y\nz");
}

// -----------------------------------------------------------------------

#[test]
fn grep_lines_filters_case_insensitive() {
    let text = "INFO starting\nERROR bad thing\nINFO ok\nError another";
    assert_eq!(grep_lines(text, "error"), "ERROR bad thing\nError another");
}

#[test]
fn grep_lines_no_match() {
    let text = "INFO starting\nINFO ok";
    assert_eq!(grep_lines(text, "error"), "");
}

#[test]
fn grep_lines_empty_input() {
    assert_eq!(grep_lines("", "error"), "");
}

#[test]
fn grep_lines_empty_pattern_matches_all() {
    let text = "line one\nline two\nline three";
    assert_eq!(grep_lines(text, ""), text);
}

#[test]
fn grep_lines_single_line_match() {
    assert_eq!(grep_lines("only line", "only"), "only line");
}

#[test]
fn grep_lines_single_line_no_match() {
    assert_eq!(grep_lines("only line", "missing"), "");
}

#[test]
fn grep_lines_all_lines_match() {
    let text = "error one\nerror two\nerror three";
    assert_eq!(grep_lines(text, "error"), text);
}

#[test]
fn grep_lines_mixed_case_pattern() {
    let text = "ERROR here\nerror there\nErrOr everywhere";
    assert_eq!(grep_lines(text, "ErRoR"), text);
}

#[test]
fn grep_lines_special_chars_literal() {
    // grep_lines does substring matching, not regex -- special chars are literal
    let text = "rate is 99.9%\nrate is 100%\nno rate here";
    assert_eq!(grep_lines(text, "99.9%"), "rate is 99.9%");
}

#[test]
fn grep_lines_regex_metacharacters_are_literal() {
    let text = "file.rs:10\nfilexrs:10\nno match";
    // "." should NOT match "x" -- it's substring, not regex
    assert_eq!(grep_lines(text, "file.rs"), "file.rs:10");
}

#[test]
fn grep_lines_brackets_literal() {
    let text = "vec[0] = 1\nvec_0 = 1\nother";
    assert_eq!(grep_lines(text, "[0]"), "vec[0] = 1");
}

#[test]
fn grep_lines_unicode() {
    let text = "normal line\nline with \u{00e9}m\u{00f8}ji\nanother";
    assert_eq!(grep_lines(text, "\u{00e9}m\u{00f8}"), "line with \u{00e9}m\u{00f8}ji");
}

#[test]
fn grep_lines_preserves_line_order() {
    let text = "c third\na first\nb second";
    assert_eq!(grep_lines(text, ""), "c third\na first\nb second");
}

#[test]
fn grep_lines_trailing_newline() {
    // A trailing newline produces an empty last line -- should not appear in output
    let text = "error here\ninfo there\n";
    assert_eq!(grep_lines(text, "error"), "error here");
}

#[test]
fn grep_lines_whitespace_pattern() {
    let text = "  indented\nnot indented\n\ttabbed";
    assert_eq!(grep_lines(text, "\t"), "\ttabbed");
}
