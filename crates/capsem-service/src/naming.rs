//! VM-name helpers: generated session names and persistent-name validation.

use anyhow::{anyhow, Result};
use rand::Rng;

/// The first free `vm-N` among `existing` names.
pub fn generate_session_name<I, S>(existing: I) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let base = "vm";
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

/// Validate that a persistent VM name (or VM label key) is safe for use as an identifier.
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
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(anyhow!(
            "VM name must contain only letters, digits, hyphens, and underscores"
        ));
    }
    Ok(())
}

/// Validate user-supplied advisory VM labels before persisting or registering a VM.
///
/// Rules:
/// - at most 64 entries
/// - keys: reuse the VM-name rule (`validate_vm_name`: `1..=64` ASCII chars starting with
///   `[A-Za-z0-9]` and containing only `[A-Za-z0-9_-]`)
/// - values: `<= 255` UTF-8 bytes with no control characters (`char::is_control`)
pub fn validate_vm_labels(labels: Option<&std::collections::HashMap<String, String>>) -> Result<()> {
    let Some(labels) = labels else {
        return Ok(());
    };
    if labels.len() > 64 {
        anyhow::bail!("too many VM labels (max 64)");
    }
    for (key, value) in labels {
        validate_vm_name(key).map_err(|reason| anyhow::anyhow!("invalid VM label key {key:?}: {reason}"))?;
        if value.len() > 255 {
            anyhow::bail!("VM label value for {key:?} too long (max 255 bytes)");
        }
        if value.chars().any(char::is_control) {
            anyhow::bail!("VM label value for {key:?} must not contain control characters");
        }
    }
    Ok(())
}

/// Normalize an optional label map so an empty map `{}` is treated identically to `None`.
pub fn non_empty_labels(
    labels: Option<std::collections::HashMap<String, String>>,
) -> Option<std::collections::HashMap<String, String>> {
    labels.filter(|labels| !labels.is_empty())
}

#[cfg(test)]
mod tests;
