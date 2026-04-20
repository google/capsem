//! VM-name helpers: human-readable temp names and persistent-name validation.

use anyhow::{anyhow, Result};
use rand::Rng;

/// Generate a fun temporary VM name like `tmp-brave-falcon`.
///
/// The shape is `tmp-<adj>-<noun>` -- two hyphens, lowercase ASCII only.
/// Callers rely on the `tmp-` prefix to distinguish auto-generated names from
/// user-supplied ones.
pub fn generate_tmp_name() -> String {
    const ADJECTIVES: &[&str] = &[
        "brave", "calm", "clever", "daring", "eager", "fancy", "gentle",
        "happy", "jolly", "keen", "lively", "lucky", "merry", "noble",
        "plucky", "quick", "quiet", "sharp", "smart", "swift", "witty",
        "zany", "bright", "bold", "proud", "fierce", "steady", "agile",
        "cosmic", "epic", "grand", "mighty", "nimble", "stellar", "vivid",
    ];
    const NOUNS: &[&str] = &[
        "phoenix", "falcon", "otter", "panda", "wolf", "tiger", "raven",
        "cobra", "dolphin", "hawk", "lynx", "puma", "fox", "owl", "bear",
        "jaguar", "eagle", "heron", "bison", "coral", "amber", "jade",
        "onyx", "ruby", "opal", "ivory", "crimson", "indigo", "violet",
        "bronze", "silver", "cedar", "maple", "willow", "aurora", "comet",
        "nova", "nebula", "summit", "ridge", "canyon", "glacier", "thunder",
        "blaze", "ember", "frost", "breeze",
    ];
    let mut rng = rand::thread_rng();
    let adj = ADJECTIVES[rng.gen_range(0..ADJECTIVES.len())];
    let noun = NOUNS[rng.gen_range(0..NOUNS.len())];
    format!("tmp-{adj}-{noun}")
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
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(anyhow!("VM name must contain only letters, digits, hyphens, and underscores"));
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
    fn generate_tmp_name_starts_with_tmp_prefix() {
        for _ in 0..32 {
            let n = generate_tmp_name();
            assert!(n.starts_with("tmp-"), "generated name {n:?} missing tmp- prefix");
        }
    }

    #[test]
    fn generate_tmp_name_has_exactly_two_hyphens() {
        for _ in 0..32 {
            let n = generate_tmp_name();
            let hyphens = n.bytes().filter(|b| *b == b'-').count();
            assert_eq!(hyphens, 2, "name {n:?} should have exactly two hyphens");
        }
    }

    #[test]
    fn generate_tmp_name_passes_validate_vm_name() {
        // Auto-generated names must pass the validator that gates persistent
        // names -- the temp-name shape doubles as a safety check on the word lists.
        for _ in 0..16 {
            let n = generate_tmp_name();
            validate_vm_name(&n).expect("generated tmp name must validate");
        }
    }
}
