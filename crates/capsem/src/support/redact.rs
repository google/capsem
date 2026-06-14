//! Redactor for support bundles. Strips secrets so a bundle can be
//! attached to a public bug report without leaking credentials.
//!
//! Five rules, in order:
//!
//! 1. **TOML/JSON keys named like a secret**: `(?i)(token|secret|api[_-]?key
//!    |password|authorization|gateway[_-]?token|github[_-]?token)` ->
//!    value replaced with `<redacted>`.
//! 2. **Bearer tokens in log lines**: `Authorization:\s*Bearer\s+\S+` ->
//!    `Authorization: Bearer <redacted>`.
//! 3. **Provider key prefixes**: `sk-...`, `AIza...`, `xox[baprs]-...` ->
//!    `<redacted-key>`.
//! 4. **Home-directory paths**: `/Users/<x>/`, `/home/<x>/` -> `~/`. Not
//!    secret per se, but reduces noise + identifies the reporter less
//!    than necessary.
//! 5. The MITM CA fingerprint is exempt (it IS a fingerprint, not the
//!    cert), but the cert itself is never bundled.
//!
//! `--no-redact` flips the redactor to a passthrough.

use std::sync::OnceLock;

use regex::Regex;

/// Public entry: redact a single line of log output.
pub fn redact_line(line: &str) -> String {
    let bearer = RE_BEARER.get_or_init(bearer_re);
    let api = RE_API_KEY.get_or_init(api_key_re);
    let home = RE_HOME_PATH.get_or_init(home_path_re);
    if !bearer.is_match(line) && !api.is_match(line) && !home.is_match(line) {
        return line.to_string();
    }
    let s = bearer.replace_all(line, "Authorization: Bearer <redacted>");
    let s = api.replace_all(&s, "<redacted-key>");
    let s = home.replace_all(&s, "~/");
    s.into_owned()
}

/// Redact a whole TOML/JSON file's text. Replaces values for any line
/// whose key matches a secret-name regex with `"<redacted>"`. Operates
/// at line granularity (TOML/JSON one-key-per-line conventions); pretty
/// blobs of multi-line nested values may slip through. Adequate for
/// the settings.toml/corp.toml shapes we ship.
pub fn redact_config_text(text: &str) -> String {
    let key_re = RE_SECRET_KEY.get_or_init(secret_key_re);
    text.lines()
        .map(|line| {
            if key_re.is_match(line) {
                // Find the `=` or `:` and replace everything after with
                // a redacted placeholder, preserving leading indent and
                // the key.
                if let Some(eq_idx) = line.find('=') {
                    format!("{}= \"<redacted>\"", &line[..eq_idx])
                } else if let Some(colon_idx) = line.find(':') {
                    format!("{}: \"<redacted>\"", &line[..colon_idx])
                } else {
                    line.to_string()
                }
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// One-time-compiled regex slots. Compiled lazily on first call; the
// Regex crate's compile cost is non-trivial so we cache. Memoized via
// OnceLock rather than `lazy_static` to keep deps minimal.
static RE_BEARER: OnceLock<Regex> = OnceLock::new();
static RE_API_KEY: OnceLock<Regex> = OnceLock::new();
static RE_HOME_PATH: OnceLock<Regex> = OnceLock::new();
static RE_SECRET_KEY: OnceLock<Regex> = OnceLock::new();

fn bearer_re() -> Regex {
    Regex::new("(?i)Authorization:\\s*Bearer\\s+\\S+").unwrap()
}

fn api_key_re() -> Regex {
    // sk- (Anthropic / OpenAI), AIza (Google), xox[baprs]- (Slack).
    // 20+ char tail is the conservative threshold to avoid eating
    // shorter unrelated strings.
    Regex::new("(sk-[A-Za-z0-9_\\-]{20,}|AIza[A-Za-z0-9_\\-]{20,}|xox[baprs]-[A-Za-z0-9_\\-]+)")
        .unwrap()
}

fn home_path_re() -> Regex {
    Regex::new("/(?:Users|home)/[^/\\s\"]+/").unwrap()
}

fn secret_key_re() -> Regex {
    // The key keyword may be wrapped in `"..."` (JSON) or unquoted (TOML),
    // followed by optional `"` and whitespace before the `=` or `:`.
    Regex::new("(?i)(?:\\b|\")(token|secret|api[_-]?key|password|authorization|gateway[_-]?token|github[_-]?token)(?:\\b|\")[\"'\\s]*[=:]")
        .unwrap()
}

#[cfg(test)]
mod tests;
