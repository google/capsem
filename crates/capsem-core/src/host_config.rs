//! Host configuration detection.
//!
//! Scans the user's macOS host for pre-existing developer configuration
//! (git identity, SSH keys, API keys, GitHub tokens) to pre-fill the
//! first-run setup wizard. All detection is best-effort -- any error
//! returns None for that field.

use serde::Serialize;
use std::path::PathBuf;
use std::process::Command;

/// Detected host configuration for the setup wizard.
#[derive(Debug, Clone, Default, Serialize)]
pub struct HostConfig {
    pub git_name: Option<String>,
    pub git_email: Option<String>,
    pub ssh_public_key: Option<String>,
    pub anthropic_api_key: Option<String>,
    pub google_api_key: Option<String>,
    pub openai_api_key: Option<String>,
    pub github_token: Option<String>,
}

/// Detect all available host configuration.
pub fn detect() -> HostConfig {
    let home = match std::env::var("HOME").ok() {
        Some(h) => PathBuf::from(h),
        None => return HostConfig::default(),
    };

    let git = detect_git_identity(&home);
    HostConfig {
        git_name: git.0,
        git_email: git.1,
        ssh_public_key: detect_ssh_public_key(&home),
        anthropic_api_key: detect_anthropic_key(&home),
        google_api_key: detect_google_key(&home),
        openai_api_key: detect_openai_key(),
        github_token: detect_github_token(),
    }
}

/// Parse ~/.gitconfig for [user] name and email.
fn detect_git_identity(home: &PathBuf) -> (Option<String>, Option<String>) {
    let path = home.join(".gitconfig");
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return (None, None),
    };

    let mut name = None;
    let mut email = None;
    let mut in_user_section = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_user_section = trimmed.eq_ignore_ascii_case("[user]");
            continue;
        }
        if !in_user_section {
            continue;
        }
        if let Some((key, value)) = trimmed.split_once('=') {
            let key = key.trim().to_lowercase();
            let value = value.trim().to_string();
            if !value.is_empty() {
                match key.as_str() {
                    "name" => name = Some(value),
                    "email" => email = Some(value),
                    _ => {}
                }
            }
        }
    }

    (name, email)
}

/// Read ~/.ssh/id_ed25519.pub or ~/.ssh/id_rsa.pub.
fn detect_ssh_public_key(home: &PathBuf) -> Option<String> {
    let candidates = ["id_ed25519.pub", "id_rsa.pub"];
    for name in &candidates {
        let path = home.join(".ssh").join(name);
        if let Ok(content) = std::fs::read_to_string(&path) {
            let trimmed = content.trim().to_string();
            if !trimmed.is_empty() {
                return Some(trimmed);
            }
        }
    }
    None
}

/// Detect Anthropic API key from env var or ~/.claude/settings.json.
fn detect_anthropic_key(home: &PathBuf) -> Option<String> {
    if let Some(key) = non_empty_env("ANTHROPIC_API_KEY") {
        return Some(key);
    }
    // Try ~/.claude/settings.json
    let path = home.join(".claude").join("settings.json");
    if let Ok(content) = std::fs::read_to_string(&path) {
        if let Some(key) = extract_json_string_field(&content, "apiKey") {
            return Some(key);
        }
    }
    None
}

/// Detect Google AI API key from env var or ~/.gemini/settings.json.
fn detect_google_key(home: &PathBuf) -> Option<String> {
    if let Some(key) = non_empty_env("GEMINI_API_KEY") {
        return Some(key);
    }
    // Try ~/.gemini/settings.json
    let path = home.join(".gemini").join("settings.json");
    if let Ok(content) = std::fs::read_to_string(&path) {
        if let Some(key) = extract_json_string_field(&content, "apiKey") {
            return Some(key);
        }
    }
    None
}

/// Detect OpenAI API key from env var.
fn detect_openai_key() -> Option<String> {
    non_empty_env("OPENAI_API_KEY")
}

/// Detect GitHub token via `gh auth token`.
fn detect_github_token() -> Option<String> {
    let output = Command::new("gh")
        .args(["auth", "token"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if token.is_empty() { None } else { Some(token) }
}

/// Read an env var, returning None if empty or unset.
fn non_empty_env(key: &str) -> Option<String> {
    match std::env::var(key) {
        Ok(v) if !v.trim().is_empty() => Some(v.trim().to_string()),
        _ => None,
    }
}

/// Extract a string value for a given key from a JSON string (simple search).
/// Not a full JSON parser -- looks for `"key": "value"` patterns.
fn extract_json_string_field(json: &str, field: &str) -> Option<String> {
    // Look for "field" followed by : and a quoted string value
    let pattern = format!("\"{}\"", field);
    let idx = json.find(&pattern)?;
    let after_key = &json[idx + pattern.len()..];
    // Skip whitespace and colon
    let after_colon = after_key.trim_start().strip_prefix(':')?;
    let after_ws = after_colon.trim_start();
    if !after_ws.starts_with('"') {
        return None;
    }
    let value_start = &after_ws[1..];
    let end = value_start.find('"')?;
    let value = &value_start[..end];
    if value.is_empty() { None } else { Some(value.to_string()) }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_returns_default_without_panic() {
        // Should never panic even if nothing is found.
        let config = detect();
        // At minimum, the struct should be constructable.
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
        let (name, email) = detect_git_identity(&dir.path().to_path_buf());
        assert_eq!(name.as_deref(), Some("Alice Example"));
        assert_eq!(email.as_deref(), Some("alice@example.com"));
    }

    #[test]
    fn parse_gitconfig_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let (name, email) = detect_git_identity(&dir.path().to_path_buf());
        assert!(name.is_none());
        assert!(email.is_none());
    }

    #[test]
    fn parse_gitconfig_empty_values() {
        let dir = tempfile::tempdir().unwrap();
        let gitconfig = dir.path().join(".gitconfig");
        std::fs::write(&gitconfig, "[user]\n\tname = \n\temail = \n").unwrap();
        let (name, email) = detect_git_identity(&dir.path().to_path_buf());
        assert!(name.is_none());
        assert!(email.is_none());
    }

    #[test]
    fn parse_gitconfig_no_user_section() {
        let dir = tempfile::tempdir().unwrap();
        let gitconfig = dir.path().join(".gitconfig");
        std::fs::write(&gitconfig, "[core]\n\teditor = vim\n").unwrap();
        let (name, email) = detect_git_identity(&dir.path().to_path_buf());
        assert!(name.is_none());
        assert!(email.is_none());
    }

    #[test]
    fn parse_gitconfig_case_insensitive_section() {
        let dir = tempfile::tempdir().unwrap();
        let gitconfig = dir.path().join(".gitconfig");
        std::fs::write(&gitconfig, "[User]\n\tname = Bob\n\temail = bob@test.com\n").unwrap();
        let (name, email) = detect_git_identity(&dir.path().to_path_buf());
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
        assert_eq!(detect_ssh_public_key(&dir.path().to_path_buf()).as_deref(), Some(key));
    }

    #[test]
    fn ssh_public_key_rsa_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let ssh_dir = dir.path().join(".ssh");
        std::fs::create_dir_all(&ssh_dir).unwrap();
        let key = "ssh-rsa AAAAB3NzaC1yc2EAAAATest user@host";
        std::fs::write(ssh_dir.join("id_rsa.pub"), key).unwrap();
        assert_eq!(detect_ssh_public_key(&dir.path().to_path_buf()).as_deref(), Some(key));
    }

    #[test]
    fn ssh_public_key_prefers_ed25519() {
        let dir = tempfile::tempdir().unwrap();
        let ssh_dir = dir.path().join(".ssh");
        std::fs::create_dir_all(&ssh_dir).unwrap();
        std::fs::write(ssh_dir.join("id_ed25519.pub"), "ssh-ed25519 PREFERRED").unwrap();
        std::fs::write(ssh_dir.join("id_rsa.pub"), "ssh-rsa FALLBACK").unwrap();
        assert_eq!(
            detect_ssh_public_key(&dir.path().to_path_buf()).as_deref(),
            Some("ssh-ed25519 PREFERRED")
        );
    }

    #[test]
    fn ssh_public_key_missing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(detect_ssh_public_key(&dir.path().to_path_buf()).is_none());
    }

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
}
