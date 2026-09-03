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

// ---------------------------------------------------------------------------
// Attacker-shaped conditions: every malformed input is an error, never a panic
// or a silently different rule.
// ---------------------------------------------------------------------------

#[test]
fn malformed_conditions_fail_to_compile_without_panicking() {
    let long = "a".repeat(1 << 20);
    let deep = format!("{}http.path == \"x\"{}", "(".repeat(2000), ")".repeat(2000));
    let cases: Vec<String> = vec![
        String::new(),
        "   ".into(),
        r#"http.path == "\"#.into(),
        r#"http.path == "\u""#.into(),
        r#"http.path == "\uD800A""#.into(),
        r#"http.path == "\x4""#.into(),
        r#"http.path == "\U110000""#.into(),
        r#"http.path == 'a" b'x"#.into(),
        r#"http.path.matches("(")"#.into(),
        r#"http.path.matches("[")"#.into(),
        r#"http.path.matches("(?P<n>")"#.into(),
        r#"http.path.matches(")")"#.into(),
        r#"http.path == "a" &&"#.into(),
        r#"|| http.path == "a""#.into(),
        r#"http.path == "a" || || http.path == "b""#.into(),
        r#"has()"#.into(),
        r#"has(http.path"#.into(),
        r#"http.path.contains()"#.into(),
        r#"http.path.contains("a", "b")"#.into(),
        r#"http.path.unknown("a")"#.into(),
        r#"http.path ~= "a""#.into(),
        r#"http.path == "a" == "b""#.into(),
        format!("http.path == \"{long}"),
    ];
    for condition in cases {
        let result = std::panic::catch_unwind(|| CompiledCondition::parse_with(&condition, |_| Ok(())));
        let shown: String = condition.chars().take(60).collect();
        let result = result.unwrap_or_else(|_| panic!("{shown:?} panicked the parser"));
        assert!(result.is_err(), "{shown:?} must not compile");
    }
    // Deep but balanced grouping is valid and must neither overflow the
    // stack nor change the meaning.
    let deep_cond = std::panic::catch_unwind(|| compile(&deep)).expect("deep grouping must not overflow");
    assert!(deep_cond.evaluate(&json!({ "http": { "path": "x" } })).unwrap());
}

#[test]
fn operators_inside_string_literals_do_not_split_the_condition() {
    assert!(path_matches(r#"http.path == "a || b""#, "a || b"));
    assert!(path_matches(r#"http.path == "a && b""#, "a && b"));
    assert!(path_matches(r#"http.path == "(a)""#, "(a)"));
    assert!(path_matches(r#"http.path == "a == b""#, "a == b"));
    assert!(path_matches(r#"http.path.contains(")")"#, "x)y"));
    assert!(!path_matches(r#"http.path == "a || b""#, "a"));
}

#[test]
fn hostile_regexes_are_bounded_by_the_engine() {
    // The regex crate guarantees linear time; a classic catastrophic pattern
    // over a long subject must still return promptly.
    let subject = format!("{}!", "a".repeat(100_000));
    let started = std::time::Instant::now();
    assert!(!path_matches(r#"http.path.matches("^(a+)+$")"#, &subject));
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
    // A very large regex either compiles and stays linear or is refused; it
    // never hangs compilation or matching.
    let huge = format!("http.path.matches(\"{}\")", "a{1000}".repeat(200));
    let started = std::time::Instant::now();
    if let Ok(cond) = CompiledCondition::parse_with(&huge, |_| Ok(())) {
        let _ = cond.evaluate(&json!({ "http": { "path": subject } }));
    }
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
}

#[test]
fn absent_and_non_string_fields_never_match_equality() {
    let cond = compile(r#"http.path == """#);
    assert!(!cond
        .evaluate(&json!({ "http": { "other": "" } }))
        .expect("absent field evaluates"));
    assert!(!cond.evaluate(&json!({ "http": { "path": 0 } })).unwrap_or(false));
    assert!(!cond.evaluate(&json!({ "http": { "path": null } })).unwrap_or(false));
}
