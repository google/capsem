//! VM-name helpers: human-readable temp names and persistent-name validation.

use anyhow::{anyhow, Result};
use rand::seq::SliceRandom;
use rand::Rng;

const ADJECTIVES: &[&str] = &[
    "agile", "ample", "bold", "bonny", "brave", "bright", "calm", "cheerful",
    "clever", "cosmic", "cozy", "crafty", "daring", "dapper", "dashing",
    "eager", "elegant", "epic", "fancy", "feisty", "fierce", "friendly",
    "gentle", "gleeful", "glossy", "grand", "happy", "hardy", "honest",
    "jazzy", "jolly", "keen", "kindly", "lively", "lofty", "lucky", "mellow",
    "merry", "mighty", "nimble", "noble", "pearly", "peppy", "placid",
    "plucky", "proud", "quick", "quiet", "royal", "rustic", "serene", "sharp",
    "sleek", "smart", "steady", "stellar", "swift", "tender", "tidy",
    "upbeat", "valiant", "vibrant", "vivid", "whimsical", "winsome", "witty",
    "zany", "zesty",
];

const NOUNS: &[&str] = &[
    "amber", "aurora", "badger", "beacon", "bear", "beaver", "bison",
    "blaze", "bobcat", "breeze", "bronze", "canyon", "cedar", "comet",
    "cobra", "coral", "cougar", "cricket", "crimson", "dolphin", "dragon",
    "eagle", "ember", "falcon", "finch", "fox", "frost", "galaxy", "gecko",
    "glacier", "griffin", "hare", "hawk", "heron", "ibis", "indigo", "ivory",
    "jade", "jaguar", "kestrel", "kiwi", "koala", "lemur", "llama", "lotus",
    "lynx", "maple", "marlin", "meadow", "meteor", "moth", "narwhal",
    "nebula", "nova", "onyx", "opal", "orchid", "osprey", "otter", "owl",
    "panda", "pebble", "phoenix", "pine", "puma", "quartz", "raven", "ridge",
    "river", "ruby", "sable", "seal", "silver", "sparrow", "spruce", "stone",
    "summit", "swan", "thunder", "tiger", "tundra", "violet", "vortex",
    "willow", "wolf", "zephyr",
];

/// Generate a fun temporary VM name like `brave-falcon-tmp`.
///
/// The shape is `<adj>-<noun>-tmp` -- two hyphens, lowercase ASCII only. The
/// `-tmp` suffix (rather than a prefix) keeps the distinctive adjective at
/// the start of tab titles and VM lists so users can tell instances apart at
/// a glance.
///
/// `existing` is an iterator over names already in use (any source -- running
/// VMs, persistent VMs, in-flight provisions). The generator avoids picking
/// an adjective whose string matches the first `-`-separated segment of any
/// existing name, so two concurrent temp VMs never share a leading word. If
/// every adjective is already claimed the function falls back to a random
/// adjective rather than failing.
pub fn generate_tmp_name<I, S>(existing: I) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let used_first_words: std::collections::HashSet<String> = existing
        .into_iter()
        .map(|name| {
            name.as_ref()
                .split('-')
                .next()
                .unwrap_or("")
                .to_ascii_lowercase()
        })
        .filter(|w| !w.is_empty())
        .collect();

    let mut rng = rand::thread_rng();

    let adj = {
        let candidates: Vec<&&str> = ADJECTIVES
            .iter()
            .filter(|a| !used_first_words.contains(**a))
            .collect();
        if let Some(pick) = candidates.choose(&mut rng) {
            **pick
        } else {
            ADJECTIVES[rng.gen_range(0..ADJECTIVES.len())]
        }
    };
    let noun = NOUNS[rng.gen_range(0..NOUNS.len())];
    format!("{adj}-{noun}-tmp")
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
    fn generate_tmp_name_ends_with_tmp_suffix() {
        for _ in 0..32 {
            let n = generate_tmp_name(std::iter::empty::<&str>());
            assert!(n.ends_with("-tmp"), "generated name {n:?} missing -tmp suffix");
            assert!(!n.starts_with("tmp-"), "generated name {n:?} must not keep tmp- prefix");
        }
    }

    #[test]
    fn generate_tmp_name_has_exactly_two_hyphens() {
        for _ in 0..32 {
            let n = generate_tmp_name(std::iter::empty::<&str>());
            let hyphens = n.bytes().filter(|b| *b == b'-').count();
            assert_eq!(hyphens, 2, "name {n:?} should have exactly two hyphens");
        }
    }

    #[test]
    fn generate_tmp_name_passes_validate_vm_name() {
        // Auto-generated names must pass the validator that gates persistent
        // names -- the temp-name shape doubles as a safety check on the word lists.
        for _ in 0..16 {
            let n = generate_tmp_name(std::iter::empty::<&str>());
            validate_vm_name(&n).expect("generated tmp name must validate");
        }
    }

    #[test]
    fn generate_tmp_name_avoids_existing_first_word() {
        // Reserve every adjective but one and confirm we pick the free one.
        let free = "zesty";
        let used: Vec<String> = ADJECTIVES
            .iter()
            .filter(|a| **a != free)
            .map(|a| format!("{a}-wolf-tmp"))
            .collect();
        for _ in 0..16 {
            let n = generate_tmp_name(used.iter().map(|s| s.as_str()));
            assert!(
                n.starts_with(&format!("{free}-")),
                "expected generator to pick the only free adjective, got {n:?}"
            );
        }
    }

    #[test]
    fn generate_tmp_name_falls_back_when_all_adjectives_used() {
        // Every adjective claimed -- the generator must still return something
        // that validates rather than panicking or spinning forever.
        let used: Vec<String> = ADJECTIVES
            .iter()
            .map(|a| format!("{a}-wolf-tmp"))
            .collect();
        let n = generate_tmp_name(used.iter().map(|s| s.as_str()));
        validate_vm_name(&n).expect("fallback name must still validate");
        assert!(n.ends_with("-tmp"));
    }

    #[test]
    fn generate_tmp_name_ignores_empty_existing() {
        // An empty iterator is the no-collision case; the prior test exercised
        // this, so this just guards against a regression where an empty string
        // in the input accidentally blocks every adjective.
        let n = generate_tmp_name(std::iter::once(""));
        validate_vm_name(&n).expect("empty existing name should not break generator");
    }
}
