//! VM-name helpers: generated session names and persistent-name validation.

use anyhow::Result;
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

/// Validate that a persistent VM name is safe for use as a directory name.
///
/// Rules:
/// - non-empty
/// - <= 64 characters
/// - starts with an ASCII letter or digit (no leading hyphen/underscore)
/// - consists only of ASCII alphanumerics, `-`, or `_`
pub fn validate_vm_name(name: &str) -> Result<()> {
    capsem_api::validate_name("VM name", name).map_err(anyhow::Error::msg)
}

#[cfg(test)]
mod tests;
