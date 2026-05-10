use super::*;

#[test]
fn bearer_token_in_log_line_is_redacted() {
    let line = r#"GET /v1/messages HTTP/1.1, Authorization: Bearer sk-ant-abcdefghijklmnopqrstuv"#;
    let r = redact_line(line);
    assert!(r.contains("Bearer <redacted>"), "{r}");
    assert!(!r.contains("sk-ant-abcdefghijklmnopqrstuv"));
}

#[test]
fn anthropic_key_prefix_is_redacted() {
    let line = "loaded ANTHROPIC_API_KEY=sk-ant-api03_abcdefghijklmnopqrstuvwxyz";
    let r = redact_line(line);
    assert!(r.contains("<redacted-key>"), "{r}");
    assert!(!r.contains("sk-ant-api03_abcdefghijklmnopqrstuvwxyz"));
}

#[test]
fn google_key_prefix_is_redacted() {
    let line = "key=AIzaSyABCDEFGHIJKLMNOPQRSTUVWXYZ-1234";
    let r = redact_line(line);
    assert!(r.contains("<redacted-key>"), "{r}");
}

#[test]
fn slack_xoxb_token_is_redacted() {
    let line = concat!(
        "Slack token=xoxb-1234567890-",
        "aBcDeFgHiJkLmNoPqRsTuVwX"
    );
    let r = redact_line(line);
    assert!(r.contains("<redacted-key>"), "{r}");
}

#[test]
fn home_path_is_normalized() {
    let line = r#"opening file at /Users/elie/git/capsem/crates/capsem/src/main.rs"#;
    let r = redact_line(line);
    assert!(r.starts_with("opening file at ~/"), "{r}");
    assert!(!r.contains("/Users/elie/"));
}

#[test]
fn lines_with_no_secrets_are_unchanged() {
    let line = "INFO capsem-process starting up";
    assert_eq!(redact_line(line), line);
}

// F10: adversarial-shape fixtures from the followups audit. Each
// asserts the redactor does NOT leak the secret even when the input is
// shaped to evade the simple pattern matchers.

#[test]
fn openai_proj_key_prefix_redacted() {
    // sk-proj-... is the modern OpenAI form; same regex catches it
    // because the literal prefix is `sk-`.
    let line = "OPENAI_API_KEY=sk-proj-abcdefghijklmnopqrstuvwxyz1234567890";
    let r = redact_line(line);
    assert!(r.contains("<redacted-key>"), "{r}");
    assert!(!r.contains("abcdefghijklmnopqrstuvwxyz1234567890"));
}

#[test]
fn bearer_with_extra_whitespace_redacted() {
    let line = "Authorization:    Bearer    sk-very-long-secret-token-12345";
    let r = redact_line(line);
    assert!(r.contains("Bearer <redacted>"), "{r}");
    assert!(!r.contains("sk-very-long-secret-token-12345"));
}

#[test]
fn lowercase_authorization_redacted() {
    // The bearer regex is case-insensitive on the `Authorization` keyword.
    let line = "authorization: Bearer sk-pleaseredactthis-12345678";
    let r = redact_line(line);
    assert!(r.contains("Bearer <redacted>"), "{r}");
}

#[test]
fn home_path_with_special_chars_collapsed() {
    let line = "/Users/jane.doe-1/project/file.rs";
    let r = redact_line(line);
    assert!(r.starts_with("~/"), "{r}");
    assert!(!r.contains("/Users/jane.doe-1/"));
}

#[test]
fn multiple_secrets_in_one_line_all_redacted() {
    let line =
        "config: ANTHROPIC_KEY=sk-ant-aaaaaaaaaaaaaaaaaaaaaa OPENAI_KEY=sk-bbbbbbbbbbbbbbbbbbbb";
    let r = redact_line(line);
    assert!(!r.contains("aaaaaaaaaaaaaaaaaaaaaa"), "{r}");
    assert!(!r.contains("bbbbbbbbbbbbbbbbbbbb"), "{r}");
}

#[test]
fn binary_garbage_does_not_panic() {
    // Random bytes that happen to contain partial regex matches must not
    // panic the redactor.
    let line = "\x00\x01\x02 sk-aa /Users/x/ \x7f\x7e";
    let _ = redact_line(line);
}

#[test]
fn toml_secret_value_is_redacted() {
    let toml = r#"[provider.anthropic]
api_key = "sk-ant-api03-real-secret-here"
endpoint = "https://api.anthropic.com"
"#;
    let r = redact_config_text(toml);
    assert!(r.contains("api_key = \"<redacted>\""), "{r}");
    assert!(r.contains("endpoint = \"https://api.anthropic.com\""));
    assert!(!r.contains("sk-ant-api03-real-secret-here"));
}

#[test]
fn json_secret_value_is_redacted() {
    let json = r#"{
  "github_token": "ghp_real_secret_here_abc123",
  "endpoint": "https://api.github.com"
}"#;
    let r = redact_config_text(json);
    assert!(r.contains("\"github_token\": \"<redacted>\""), "{r}");
    assert!(r.contains("\"endpoint\""));
}
