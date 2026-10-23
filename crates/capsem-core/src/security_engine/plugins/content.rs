//! Bounded, compile-once content inspection. Detectors never return raw matches.
//! HTTP transport collects complete bounded bodies before enforcement when enabled.
use std::sync::LazyLock;

use capsem_config::{ContentFilterConfig, SecurityPluginConfig, SecurityPluginMode};
use regex::bytes::Regex;
use serde::Deserialize;

use crate::credential_broker::{credential_provider_for_request, detect_http_credential_with_provider};
use crate::security_engine::{
    SecurityActionError, SecurityEvent, SecurityPlugin, SecurityPluginResult, SecurityPluginStage,
};

#[derive(Deserialize)]
struct DetectorSource {
    detectors: Vec<DetectorSpec>,
}
#[derive(Deserialize)]
struct DetectorSpec {
    id: String,
    pattern: String,
    entropy: f64,
}
struct Detector {
    id: String,
    pattern: Regex,
    entropy: f64,
}

static DETECTORS: LazyLock<Vec<Detector>> = LazyLock::new(|| {
    let source: DetectorSource = toml::from_str(include_str!("detectors.toml")).expect("reviewed detector data");
    source
        .detectors
        .into_iter()
        .map(|rule| Detector {
            id: rule.id,
            pattern: Regex::new(&rule.pattern).expect("reviewed Rust-compatible detector"),
            entropy: rule.entropy,
        })
        .collect()
});

pub(in crate::security_engine) struct PiiFilterPlugin;
pub(in crate::security_engine) struct SecretExfiltrationPlugin;

impl SecurityPlugin for PiiFilterPlugin {
    fn id(&self) -> &'static str {
        "pii_filter"
    }
    fn stage(&self) -> SecurityPluginStage {
        SecurityPluginStage::Preprocess
    }
    fn apply(
        &self,
        event: SecurityEvent,
        config: SecurityPluginConfig,
    ) -> Result<SecurityPluginResult, SecurityActionError> {
        inspect_event(event, config, false)
    }
}
impl SecurityPlugin for SecretExfiltrationPlugin {
    fn id(&self) -> &'static str {
        "secret_exfiltration"
    }
    fn stage(&self) -> SecurityPluginStage {
        SecurityPluginStage::Preprocess
    }
    fn apply(
        &self,
        event: SecurityEvent,
        config: SecurityPluginConfig,
    ) -> Result<SecurityPluginResult, SecurityActionError> {
        inspect_event(event, config, true)
    }
}

fn inspect_event(
    mut event: SecurityEvent,
    config: SecurityPluginConfig,
    secrets: bool,
) -> Result<SecurityPluginResult, SecurityActionError> {
    let Some(http) = event.http.as_ref() else {
        return Ok(SecurityPluginResult::skipped(event));
    };
    let inbound = http.status.is_some();
    // Secret exfiltration is an outbound boundary. Inbound keys may be legitimate
    // OAuth exchanges; credential_broker remains their owner.
    if (secrets && inbound) || !config.content.direction.includes(inbound) {
        return Ok(SecurityPluginResult::skipped(event));
    }
    let rewrite = matches!(config.mode, SecurityPluginMode::Rewrite);
    let mut applied = false;
    if let Some(http) = event.http.as_mut() {
        if let Some(body) = http.body.as_mut() {
            applied |= inspect_string(body, config.content, secrets, rewrite);
        }
        if !inbound {
            if let Some(path) = http.path.as_mut() {
                applied |= inspect_string(path, config.content, secrets, rewrite);
            }
            if let Some(query) = http.query.as_mut() {
                applied |= inspect_string(query, config.content, secrets, rewrite);
            }
        }
    }
    if let Some(request) = event.http_request.as_mut() {
        for (name, value) in request.headers.iter_mut() {
            // Only the broker's recognized provider + destination + credential
            // slot is authorized. A token in a custom header/body is not.
            if secrets
                && (detect_http_credential_with_provider(
                    &request.domain,
                    request.ai_provider,
                    name.as_str(),
                    value.as_bytes(),
                )
                .is_some()
                    || (matches!(name.as_str(), "authorization" | "x-api-key" | "x-goog-api-key")
                        && value.to_str().ok().is_some_and(|value| {
                            capsem_config::is_credential_reference(value.strip_prefix("Bearer ").unwrap_or(value))
                        })
                        && credential_provider_for_request(&request.domain, request.ai_provider).is_some()))
            {
                continue;
            }
            if let Some(updated) = inspect_bytes(value.as_bytes(), config.content, secrets, rewrite) {
                applied = true;
                if rewrite {
                    *value = http::HeaderValue::from_bytes(&updated)
                        .map_err(|_| SecurityActionError::new("content filter produced invalid header"))?;
                }
            }
        }
        // Runtime query is separate from the CEL/ledger projection.
        if let Some(query) = request.query.as_mut() {
            applied |= inspect_string(query, config.content, secrets, rewrite);
        }
    }
    if rewrite && applied {
        if let (Some(http), Some(model)) = (event.http.as_ref(), event.model.as_mut()) {
            if inbound {
                model.response_body.clone_from(&http.body);
            } else {
                model.request_body.clone_from(&http.body);
            }
        }
    }
    if applied {
        Ok(SecurityPluginResult::applied(event))
    } else {
        Ok(SecurityPluginResult::skipped(event))
    }
}

fn inspect_string(value: &mut String, options: ContentFilterConfig, secrets: bool, rewrite: bool) -> bool {
    let Some(updated) = inspect_bytes(value.as_bytes(), options, secrets, rewrite) else {
        return false;
    };
    if rewrite {
        *value = String::from_utf8(updated).expect("masking emits valid UTF-8");
    }
    true
}

fn is_pii(id: &str) -> bool {
    matches!(id, "email" | "phone" | "card" | "ssn")
}

/// Small derived identifiers still need privacy checks when their parent
/// payload matched; ordinary model/tool identities must remain useful for stats.
pub(crate) fn sensitive_metadata(text: &str) -> bool {
    inspect_bytes(text.as_bytes(), ContentFilterConfig::default(), false, false).is_some()
        || inspect_bytes(text.as_bytes(), ContentFilterConfig::default(), true, false).is_some()
}
fn selected(rule: &Detector, options: ContentFilterConfig, secrets: bool) -> bool {
    if secrets {
        return !is_pii(&rule.id);
    }
    match rule.id.as_str() {
        "email" => options.email,
        "phone" => options.phone,
        "card" => options.card,
        "ssn" => options.ssn,
        _ => false,
    }
}

/// Allocate only on a match. No full payload copies on the no-match path.
fn inspect_bytes(bytes: &[u8], options: ContentFilterConfig, secrets: bool, rewrite: bool) -> Option<Vec<u8>> {
    let mut replacement: Option<Vec<u8>> = None;
    for rule in DETECTORS.iter().filter(|rule| selected(rule, options, secrets)) {
        for captures in rule.pattern.captures_iter(bytes) {
            let found = if secrets {
                captures.get(1).or_else(|| captures.get(0))
            } else {
                captures.get(0)
            }
            .expect("whole match");
            let value = found.as_bytes();
            if !valid_match(&rule.id, value) || (rule.entropy > 0.0 && entropy(value) < rule.entropy) {
                continue;
            }
            if !rewrite {
                return Some(Vec::new());
            }
            let masked = replacement.get_or_insert_with(|| bytes.to_vec());
            // An unquoted JSON integer must remain an integer after masking.
            let fill = if value.iter().all(u8::is_ascii_digit) {
                b'1'
            } else {
                b'*'
            };
            masked[found.range()].fill(fill);
        }
    }
    replacement
}

fn valid_match(id: &str, value: &[u8]) -> bool {
    if id == "aws-access-token" && value.ends_with(b"EXAMPLE") {
        return false;
    }
    if id == "card" {
        let digits: Vec<u8> = value.iter().filter(|c| c.is_ascii_digit()).map(|c| c - b'0').collect();
        return (13..=19).contains(&digits.len())
            && digits
                .iter()
                .rev()
                .enumerate()
                .map(|(i, d)| {
                    let n = if i % 2 == 1 { d * 2 } else { *d };
                    u32::from(if n > 9 { n - 9 } else { n })
                })
                .sum::<u32>()
                % 10
                == 0;
    }
    if id == "ssn" {
        let digits: Vec<u8> = value.iter().copied().filter(u8::is_ascii_digit).collect();
        return digits.len() == 9
            && &digits[..3] != b"000"
            && &digits[..3] != b"666"
            && digits[0] != b'9'
            && &digits[3..5] != b"00"
            && &digits[5..] != b"0000"
            && !matches!(digits.as_slice(), b"123456789" | b"987654320" | b"078051120")
            && value[3] == value[6];
    }
    true
}

fn entropy(value: &[u8]) -> f64 {
    let mut counts = [0_u32; 256];
    for byte in value {
        counts[*byte as usize] += 1;
    }
    counts
        .into_iter()
        .filter(|n| *n != 0)
        .map(|n| {
            let p = f64::from(n) / value.len() as f64;
            -p * p.log2()
        })
        .sum()
}

#[cfg(test)]
mod tests;
