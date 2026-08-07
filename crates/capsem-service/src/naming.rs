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
mod tests;
