use super::*;

#[test]
fn detect_returns_default_without_panic() {
    let config = detect();
    assert!(config.git_name.is_some() || config.git_name.is_none());
}

#[test]
fn parse_gitconfig_user_section() {
    let dir = tempfile::tempdir().unwrap();
    let gitconfig = dir.path().join(".gitconfig");
    std::fs::write(
        &gitconfig,
        "[user]\n\tname = Alice Example\n\temail = alice@example.com\n[core]\n\teditor = vim\n",
    )
    .unwrap();
    let (name, email) = detect_git_identity(dir.path());
    assert_eq!(name.as_deref(), Some("Alice Example"));
    assert_eq!(email.as_deref(), Some("alice@example.com"));
}

#[test]
fn parse_gitconfig_missing_file() {
    let dir = tempfile::tempdir().unwrap();
    let (name, email) = detect_git_identity(dir.path());
    assert!(name.is_none());
    assert!(email.is_none());
}

#[test]
fn parse_gitconfig_empty_values() {
    let dir = tempfile::tempdir().unwrap();
    let gitconfig = dir.path().join(".gitconfig");
    std::fs::write(&gitconfig, "[user]\n\tname = \n\temail = \n").unwrap();
    let (name, email) = detect_git_identity(dir.path());
    assert!(name.is_none());
    assert!(email.is_none());
}

#[test]
fn parse_gitconfig_no_user_section() {
    let dir = tempfile::tempdir().unwrap();
    let gitconfig = dir.path().join(".gitconfig");
    std::fs::write(&gitconfig, "[core]\n\teditor = vim\n").unwrap();
    let (name, email) = detect_git_identity(dir.path());
    assert!(name.is_none());
    assert!(email.is_none());
}

#[test]
fn parse_gitconfig_case_insensitive_section() {
    let dir = tempfile::tempdir().unwrap();
    let gitconfig = dir.path().join(".gitconfig");
    std::fs::write(&gitconfig, "[User]\n\tname = Bob\n\temail = bob@test.com\n").unwrap();
    let (name, email) = detect_git_identity(dir.path());
    assert_eq!(name.as_deref(), Some("Bob"));
    assert_eq!(email.as_deref(), Some("bob@test.com"));
}

#[test]
fn ssh_public_key_ed25519() {
    let dir = tempfile::tempdir().unwrap();
    let ssh_dir = dir.path().join(".ssh");
    std::fs::create_dir_all(&ssh_dir).unwrap();
    let key = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAITest user@host";
    std::fs::write(ssh_dir.join("id_ed25519.pub"), key).unwrap();
    assert_eq!(detect_ssh_public_key(dir.path()).as_deref(), Some(key));
}

#[test]
fn ssh_public_key_rsa_fallback() {
    let dir = tempfile::tempdir().unwrap();
    let ssh_dir = dir.path().join(".ssh");
    std::fs::create_dir_all(&ssh_dir).unwrap();
    let key = "ssh-rsa AAAAB3NzaC1yc2EAAAATest user@host";
    std::fs::write(ssh_dir.join("id_rsa.pub"), key).unwrap();
    assert_eq!(detect_ssh_public_key(dir.path()).as_deref(), Some(key));
}

#[test]
fn ssh_public_key_ecdsa() {
    let dir = tempfile::tempdir().unwrap();
    let ssh_dir = dir.path().join(".ssh");
    std::fs::create_dir_all(&ssh_dir).unwrap();
    let key = "ecdsa-sha2-nistp256 AAAAE2VjZHNhTest user@host";
    std::fs::write(ssh_dir.join("id_ecdsa.pub"), key).unwrap();
    assert_eq!(detect_ssh_public_key(dir.path()).as_deref(), Some(key));
}

#[test]
fn ssh_public_key_prefers_ed25519() {
    let dir = tempfile::tempdir().unwrap();
    let ssh_dir = dir.path().join(".ssh");
    std::fs::create_dir_all(&ssh_dir).unwrap();
    std::fs::write(ssh_dir.join("id_ed25519.pub"), "ssh-ed25519 PREFERRED").unwrap();
    std::fs::write(ssh_dir.join("id_ecdsa.pub"), "ecdsa-sha2-nistp256 SECOND").unwrap();
    std::fs::write(ssh_dir.join("id_rsa.pub"), "ssh-rsa FALLBACK").unwrap();
    assert_eq!(
        detect_ssh_public_key(dir.path()).as_deref(),
        Some("ssh-ed25519 PREFERRED")
    );
}

#[test]
fn ssh_public_key_missing() {
    let dir = tempfile::tempdir().unwrap();
    assert!(detect_ssh_public_key(dir.path()).is_none());
}

// -- Claude OAuth detection --

#[test]
fn detect_claude_oauth_valid() {
    let dir = tempfile::tempdir().unwrap();
    let claude_dir = dir.path().join(".claude");
    std::fs::create_dir_all(&claude_dir).unwrap();
    let creds = r#"{"claudeAiOauth":{"accessToken":"sk-ant-oat01-test","refreshToken":"sk-ant-ort01-test","expiresAt":9999999999}}"#;
    std::fs::write(claude_dir.join(".credentials.json"), creds).unwrap();
    assert_eq!(detect_claude_oauth(dir.path()).as_deref(), Some(creds));
}

#[test]
fn detect_claude_oauth_missing() {
    let dir = tempfile::tempdir().unwrap();
    assert!(detect_claude_oauth(dir.path()).is_none());
}

#[test]
fn detect_claude_oauth_no_refresh_token() {
    let dir = tempfile::tempdir().unwrap();
    let claude_dir = dir.path().join(".claude");
    std::fs::create_dir_all(&claude_dir).unwrap();
    std::fs::write(claude_dir.join(".credentials.json"), r#"{"claudeAiOauth":{}}"#).unwrap();
    assert!(detect_claude_oauth(dir.path()).is_none());
}

// -- Google ADC detection --

#[test]
fn detect_google_adc_valid() {
    let dir = tempfile::tempdir().unwrap();
    let gcloud_dir = dir.path().join(".config").join("gcloud");
    std::fs::create_dir_all(&gcloud_dir).unwrap();
    let adc = r#"{"type":"authorized_user","client_id":"x","client_secret":"y","refresh_token":"z"}"#;
    std::fs::write(gcloud_dir.join("application_default_credentials.json"), adc).unwrap();
    assert_eq!(detect_google_adc(dir.path()).as_deref(), Some(adc));
}

#[test]
fn detect_google_adc_missing() {
    let dir = tempfile::tempdir().unwrap();
    assert!(detect_google_adc(dir.path()).is_none());
}

#[test]
fn detect_google_adc_no_refresh_token() {
    let dir = tempfile::tempdir().unwrap();
    let gcloud_dir = dir.path().join(".config").join("gcloud");
    std::fs::create_dir_all(&gcloud_dir).unwrap();
    std::fs::write(gcloud_dir.join("application_default_credentials.json"), r#"{"type":"service_account"}"#).unwrap();
    assert!(detect_google_adc(dir.path()).is_none());
}

// -- read_key_file tests --

#[test]
fn read_key_file_reads_content() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("key");
    std::fs::write(&path, "sk-test-123\n").unwrap();
    assert_eq!(read_key_file(&path).as_deref(), Some("sk-test-123"));
}

#[test]
fn read_key_file_empty_returns_none() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("key");
    std::fs::write(&path, "   \n  ").unwrap();
    assert!(read_key_file(&path).is_none());
}

#[test]
fn read_key_file_missing_returns_none() {
    assert!(read_key_file(Path::new("/nonexistent/path/key")).is_none());
}

// -- OpenAI config file detection --

#[test]
fn detect_openai_key_from_config_file() {
    let dir = tempfile::tempdir().unwrap();
    let key_dir = dir.path().join(".config").join("openai");
    std::fs::create_dir_all(&key_dir).unwrap();
    std::fs::write(key_dir.join("api_key"), "sk-openai-from-file\n").unwrap();
    assert_eq!(
        detect_openai_key(dir.path()).as_deref(),
        Some("sk-openai-from-file")
    );
}

#[test]
fn detect_openai_key_empty_file_returns_none() {
    let dir = tempfile::tempdir().unwrap();
    let key_dir = dir.path().join(".config").join("openai");
    std::fs::create_dir_all(&key_dir).unwrap();
    std::fs::write(key_dir.join("api_key"), "  \n").unwrap();
    assert!(detect_openai_key(dir.path()).is_none());
}

// -- Anthropic SDK file detection --

#[test]
fn detect_anthropic_key_from_sdk_file() {
    let dir = tempfile::tempdir().unwrap();
    let key_dir = dir.path().join(".anthropic");
    std::fs::create_dir_all(&key_dir).unwrap();
    std::fs::write(key_dir.join("api_key"), "sk-ant-sdk-key\n").unwrap();
    assert_eq!(
        detect_anthropic_key(dir.path()).as_deref(),
        Some("sk-ant-sdk-key")
    );
}

#[test]
fn detect_anthropic_key_empty_sdk_returns_none() {
    let dir = tempfile::tempdir().unwrap();
    let key_dir = dir.path().join(".anthropic");
    std::fs::create_dir_all(&key_dir).unwrap();
    std::fs::write(key_dir.join("api_key"), "   \n").unwrap();
    assert!(detect_anthropic_key(dir.path()).is_none());
}

#[test]
fn detect_anthropic_key_priority() {
    // ~/.claude/settings.json should take priority over ~/.anthropic/api_key.
    let dir = tempfile::tempdir().unwrap();
    // Set up both sources
    let claude_dir = dir.path().join(".claude");
    std::fs::create_dir_all(&claude_dir).unwrap();
    std::fs::write(
        claude_dir.join("settings.json"),
        r#"{"apiKey": "sk-ant-from-claude"}"#,
    )
    .unwrap();
    let anthropic_dir = dir.path().join(".anthropic");
    std::fs::create_dir_all(&anthropic_dir).unwrap();
    std::fs::write(anthropic_dir.join("api_key"), "sk-ant-from-sdk\n").unwrap();
    // Claude settings.json should win
    assert_eq!(
        detect_anthropic_key(dir.path()).as_deref(),
        Some("sk-ant-from-claude")
    );
}

// -- JSON extraction --

#[test]
fn extract_json_string_basic() {
    let json = r#"{"apiKey": "sk-ant-test123", "other": "val"}"#;
    assert_eq!(
        extract_json_string_field(json, "apiKey").as_deref(),
        Some("sk-ant-test123")
    );
}

#[test]
fn extract_json_string_missing_key() {
    let json = r#"{"other": "val"}"#;
    assert!(extract_json_string_field(json, "apiKey").is_none());
}

#[test]
fn extract_json_string_empty_value() {
    let json = r#"{"apiKey": ""}"#;
    assert!(extract_json_string_field(json, "apiKey").is_none());
}

#[test]
fn extract_json_string_number_value() {
    let json = r#"{"apiKey": 42}"#;
    assert!(extract_json_string_field(json, "apiKey").is_none());
}

#[test]
fn extract_json_string_trims_whitespace() {
    let json = r#"{"apiKey": " sk-ant-padded "}"#;
    assert_eq!(
        extract_json_string_field(json, "apiKey").as_deref(),
        Some("sk-ant-padded")
    );
}

// -- env var tests --

#[test]
fn non_empty_env_returns_none_for_unset() {
    assert!(non_empty_env("CAPSEM_TEST_NONEXISTENT_VAR_12345").is_none());
}

#[test]
fn non_empty_env_returns_none_for_empty() {
    std::env::set_var("CAPSEM_TEST_EMPTY_VAR", "");
    assert!(non_empty_env("CAPSEM_TEST_EMPTY_VAR").is_none());
    std::env::remove_var("CAPSEM_TEST_EMPTY_VAR");
}

#[test]
fn non_empty_env_returns_value() {
    std::env::set_var("CAPSEM_TEST_HAS_VAR", "hello");
    assert_eq!(non_empty_env("CAPSEM_TEST_HAS_VAR").as_deref(), Some("hello"));
    std::env::remove_var("CAPSEM_TEST_HAS_VAR");
}

#[test]
fn non_empty_env_trims_whitespace() {
    std::env::set_var("CAPSEM_TEST_WS_VAR", "  trimmed  ");
    assert_eq!(non_empty_env("CAPSEM_TEST_WS_VAR").as_deref(), Some("trimmed"));
    std::env::remove_var("CAPSEM_TEST_WS_VAR");
}

// -- validate_api_key tests --

#[tokio::test]
async fn validate_empty_key() {
    let result = validate_api_key("anthropic", "").await.unwrap();
    assert!(!result.valid);
    assert_eq!(result.message, "API key is empty");
}

#[tokio::test]
async fn validate_whitespace_key() {
    let result = validate_api_key("google", "   ").await.unwrap();
    assert!(!result.valid);
    assert_eq!(result.message, "API key is empty");
}

#[tokio::test]
async fn validate_quoted_key_stripped() {
    // Surrounding quotes should be stripped -- the bogus key inside should
    // still reach the endpoint and get rejected, not treated as empty.
    let result = validate_api_key("anthropic", "\"sk-ant-bogus\"").await.unwrap();
    assert!(!result.valid);
    assert_eq!(result.message, "Invalid API key");
}

#[tokio::test]
async fn validate_only_quotes_is_empty() {
    let result = validate_api_key("anthropic", "\"\"").await.unwrap();
    assert!(!result.valid);
    assert_eq!(result.message, "API key is empty");
}

#[tokio::test]
async fn validate_unknown_provider() {
    let result = validate_api_key("foo", "some-key").await;
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("unknown provider"));
}

#[tokio::test]
async fn validate_anthropic_key_invalid() {
    let result = validate_api_key("anthropic", "sk-ant-bogus").await.unwrap();
    assert!(!result.valid);
    assert_eq!(result.message, "Invalid API key");
}

#[tokio::test]
async fn validate_google_key_invalid() {
    let result = validate_api_key("google", "bogus-key").await.unwrap();
    assert!(!result.valid);
}

#[tokio::test]
async fn validate_openai_key_invalid() {
    let result = validate_api_key("openai", "sk-bogus").await.unwrap();
    assert!(!result.valid);
    assert_eq!(result.message, "Invalid API key");
}

#[tokio::test]
async fn validate_github_token_invalid() {
    let result = validate_api_key("github", "ghp_bogus").await.unwrap();
    assert!(!result.valid);
    assert_eq!(result.message, "Invalid API key");
}

// Real-key validation tests -- skipped when credentials are unavailable.

/// Read a setting value from `<capsem_home>/user.toml` by dotted setting id.
/// e.g. "repository.providers.github.token" looks up
/// [settings."repository.providers.github.token"] -> value
fn read_user_toml_setting(id: &str) -> Option<String> {
    let path = crate::paths::capsem_home_opt()?.join("user.toml");
    let content = std::fs::read_to_string(path).ok()?;
    let doc: toml::Value = content.parse().ok()?;
    let settings = doc.get("settings")?;
    let entry = settings.get(id)?;
    let value = entry.get("value")?.as_str()?;
    if value.is_empty() { None } else { Some(value.to_string()) }
}

/// Try env var first, then user.toml setting.
fn real_key(env_var: &str, toml_id: &str) -> Option<String> {
    if let Ok(k) = std::env::var(env_var) {
        if !k.is_empty() {
            return Some(k);
        }
    }
    read_user_toml_setting(toml_id)
}

#[tokio::test]
async fn validate_anthropic_key_real() {
    let key = match real_key("ANTHROPIC_API_KEY", "ai.anthropic.api_key") {
        Some(k) => k,
        None => return,
    };
    let result = validate_api_key("anthropic", &key).await.unwrap();
    assert!(result.valid, "expected valid, got: {}", result.message);
}

#[tokio::test]
async fn validate_google_key_real() {
    let key = match real_key("GEMINI_API_KEY", "ai.google.api_key") {
        Some(k) => k,
        None => return,
    };
    let result = validate_api_key("google", &key).await.unwrap();
    assert!(result.valid, "expected valid, got: {}", result.message);
}

#[tokio::test]
async fn validate_openai_key_real() {
    let key = match real_key("OPENAI_API_KEY", "ai.openai.api_key") {
        Some(k) => k,
        None => return,
    };
    let result = validate_api_key("openai", &key).await.unwrap();
    assert!(result.valid, "expected valid, got: {}", result.message);
}

#[tokio::test]
async fn validate_github_token_real() {
    // Only use env var -- tokens stored in user.toml can expire silently,
    // causing spurious test failures.
    let key = match std::env::var("GITHUB_TOKEN").ok().filter(|k| !k.is_empty()) {
        Some(k) => k,
        None => return,
    };
    let result = validate_api_key("github", &key).await.unwrap();
    assert!(result.valid, "expected valid, got: {}", result.message);
}
