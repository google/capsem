use serde_json::json;

use super::*;

fn compile(condition: &str) -> CompiledCondition {
    CompiledCondition::parse_with(condition, |_| Ok(())).unwrap_or_else(|e| panic!("{condition} must compile: {e}"))
}

fn path_matches(condition: &str, path: &str) -> bool {
    compile(condition)
        .evaluate(&json!({ "http": { "path": path } }))
        .expect("evaluates")
}

// Rule generators emit JSON-escaped literals (`cel_string_literal`,
// `cel_string`), and CEL itself defines the same escapes. The parser kept the
// backslashes in the compared value, so every rule whose value held a quote,
// a backslash, or a control character compiled cleanly and never matched --
// a block rule that fails open.

#[test]
fn escaped_quotes_compare_against_the_unescaped_value() {
    assert!(path_matches(r#"http.path == "say \"hi\"""#, r#"say "hi""#));
    assert!(path_matches(r"http.path == 'it\'s'", "it's"));
    assert!(!path_matches(r#"http.path == "say \"hi\"""#, r#"say \"hi\""#));
}

#[test]
fn escaped_backslashes_compare_against_a_single_backslash() {
    assert!(path_matches(
        r#"http.path == "C:\\Windows\\cmd.exe""#,
        r"C:\Windows\cmd.exe"
    ));
}

#[test]
fn control_and_unicode_escapes_decode() {
    assert!(path_matches(r#"http.path == "a\nb""#, "a\nb"));
    assert!(path_matches(r#"http.path == "a\tb\r""#, "a\tb\r"));
    assert!(path_matches(r#"http.path == "\u0041\x42""#, "AB"));
    assert!(path_matches(r#"http.path == "\ud83d\ude00""#, "😀"));
    assert!(path_matches(r#"http.path == "\U0001F600""#, "😀"));
}

#[test]
fn json_encoded_literals_round_trip_through_every_string_operator() {
    for value in [
        r#"say "hi""#,
        r"C:\Windows\cmd.exe",
        "line one\nline two",
        "tab\there",
        "\u{1}control",
        "plain",
    ] {
        let literal = serde_json::to_string(value).unwrap();
        for condition in [
            format!("http.path == {literal}"),
            format!("http.path.contains({literal})"),
            format!("http.path.startsWith({literal})"),
            format!("http.path.endsWith({literal})"),
        ] {
            assert!(path_matches(&condition, value), "{condition} must match {value:?}");
        }
        assert!(
            !path_matches(&format!("http.path != {literal}"), value),
            "{literal} must equal its own value"
        );
    }
}

#[test]
fn regex_escapes_reach_the_regex_engine_decoded() {
    // `\\.` in the CEL literal is `\.` to the regex: a literal dot, not a
    // literal backslash followed by anything. The shipped rules write the
    // same regex as `\.`; both spellings must mean the same thing.
    for condition in [
        r#"http.host.matches("(^|.*\\.)evil\\.example$")"#,
        r#"http.host.matches("(^|.*\.)evil\.example$")"#,
    ] {
        let host_matches = |host: &str| {
            compile(condition)
                .evaluate(&json!({ "http": { "host": host } }))
                .expect("evaluates")
        };
        assert!(host_matches("evil.example"), "{condition}");
        assert!(host_matches("a.evil.example"), "{condition}");
        assert!(!host_matches("evilXexample"), "{condition}");
        assert!(!host_matches("a-evil.example"), "{condition}");
        assert!(!host_matches(r"evil\Xexample"), "{condition}");
    }
}

#[test]
fn regex_class_escapes_are_kept_verbatim_for_the_regex_engine() {
    let host_matches = |condition: &str, host: &str| {
        compile(condition)
            .evaluate(&json!({ "http": { "host": host } }))
            .expect("evaluates")
    };
    assert!(host_matches(r#"http.host.matches("^\d+\.\d+$")"#, "1.2"));
    assert!(!host_matches(r#"http.host.matches("^\d+\.\d+$")"#, "1x2"));
    // `\b` is a word boundary to the regex engine, never a backspace.
    assert!(host_matches(r#"http.host.matches("\bword\b")"#, "a word here"));
    assert!(!host_matches(r#"http.host.matches("\bword\b")"#, "swordfish"));
    // Outside a regex an unknown escape is simply the two characters.
    assert!(path_matches(r#"http.path == "\q""#, r"\q"));
}

#[test]
fn malformed_unicode_escapes_are_compile_errors() {
    for condition in [
        r#"http.path == "\u12""#,
        r#"http.path == "\ud83d""#,
        r#"http.path == "\xZZ""#,
    ] {
        let err = CompiledCondition::parse_with(condition, |_| Ok(())).expect_err(condition);
        assert!(err.contains("escape"), "{condition}: {err}");
    }
}

#[test]
fn unterminated_and_trailing_literals_still_fail() {
    assert!(CompiledCondition::parse_with(r#"http.path == "open"#, |_| Ok(())).is_err());
    assert!(CompiledCondition::parse_with(r#"http.path == "a" b"#, |_| Ok(())).is_err());
    assert!(CompiledCondition::parse_with(r#"http.path == "esc\""#, |_| Ok(())).is_err());
}
