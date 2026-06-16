//! VM-name helpers: profile-scoped session names and persistent-name validation.

use anyhow::{anyhow, Result};
use rand::Rng;

pub fn generate_profile_session_name<I, S>(profile_id: &str, existing: I) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let base = sanitize_profile_prefix(profile_id);
    let existing: std::collections::HashSet<String> = existing
        .into_iter()
        .map(|name| name.as_ref().to_ascii_lowercase())
        .collect();
    for index in 1..10_000 {
        let candidate = format!("{base}-{index}");
        if !existing.contains(&candidate) {
            return candidate;
        }
    }
    format!("{base}-{}", rand::thread_rng().gen_range(10_000..99_999))
}

fn sanitize_profile_prefix(profile_id: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in profile_id.trim().to_ascii_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            last_dash = false;
        } else if !last_dash && !out.is_empty() {
            out.push('-');
            last_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "session".to_string()
    } else {
        out
    }
}

/// Validate that a persistent VM name is safe for use as a directory name.
///
/// Rules:
/// - non-empty
/// - <= 64 characters
/// - starts with an ASCII letter or digit (no leading hyphen/underscore)
/// - consists only of ASCII alphanumerics, `-`, or `_`
pub fn validate_vm_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(anyhow!("VM name cannot be empty"));
    }
    if name.len() > 64 {
        return Err(anyhow!("VM name too long (max 64 characters)"));
    }
    if !name.chars().next().unwrap().is_ascii_alphanumeric() {
        return Err(anyhow!("VM name must start with a letter or digit"));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(anyhow!(
            "VM name must contain only letters, digits, hyphens, and underscores"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- validate_vm_name (moved from main.rs) ----

    #[test]
    fn validate_vm_name_valid() {
        assert!(validate_vm_name("my-vm").is_ok());
        assert!(validate_vm_name("project_alpha").is_ok());
        assert!(validate_vm_name("vm123").is_ok());
        assert!(validate_vm_name("a").is_ok());
    }

    #[test]
    fn validate_vm_name_empty() {
        assert!(validate_vm_name("").is_err());
    }

    #[test]
    fn validate_vm_name_path_separator() {
        assert!(validate_vm_name("my/vm").is_err());
        assert!(validate_vm_name("../escape").is_err());
    }

    #[test]
    fn validate_vm_name_starts_with_hyphen() {
        assert!(validate_vm_name("-foo").is_err());
    }

    #[test]
    fn validate_vm_name_spaces() {
        assert!(validate_vm_name("my vm").is_err());
    }

    #[test]
    fn validate_vm_name_too_long() {
        let long = "a".repeat(65);
        assert!(validate_vm_name(&long).is_err());
        let max = "a".repeat(64);
        assert!(validate_vm_name(&max).is_ok());
    }

    // ---- new tests ----

    #[test]
    fn validate_vm_name_starts_with_underscore() {
        assert!(validate_vm_name("_foo").is_err());
    }

    #[test]
    fn validate_vm_name_starts_with_digit_ok() {
        assert!(validate_vm_name("9lives").is_ok());
    }

    #[test]
    fn validate_vm_name_rejects_non_ascii() {
        // Non-ASCII letters are allowed by `char::is_alphanumeric` but NOT by
        // `is_ascii_alphanumeric`, so the validator should reject them.
        assert!(validate_vm_name("nai\u{00ef}ve").is_err());
        assert!(validate_vm_name("\u{4e2d}").is_err());
    }

    #[test]
    fn validate_vm_name_rejects_dot() {
        assert!(validate_vm_name("my.vm").is_err());
    }

    #[test]
    fn generate_profile_session_name_uses_profile_counter() {
        assert_eq!(
            generate_profile_session_name("code", std::iter::empty::<&str>()),
            "code-1"
        );
        assert_eq!(
            generate_profile_session_name("code", ["code-1", "co-work-1"]),
            "code-2"
        );
    }

    #[test]
    fn generate_profile_session_name_sanitizes_profile_id() {
        assert_eq!(
            generate_profile_session_name("Co Work!", std::iter::empty::<&str>()),
            "co-work-1"
        );
        assert_eq!(
            generate_profile_session_name("!!!", std::iter::empty::<&str>()),
            "session-1"
        );
    }
}
