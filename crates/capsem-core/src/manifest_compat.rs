//! Manifest hash extraction logic shared between build.rs and runtime.
//!
//! Supports v2 manifest format:
//! ```json
//! {"format": 2, "assets": {"current": "...", "releases": {"...": {"arches": {"arm64": {"vmlinuz": {"hash": "...", "size": 0}}}}}}}
//! ```

use std::collections::HashMap;

/// Extract asset hashes from a manifest JSON Value for a given architecture.
///
/// Returns a map of logical asset name to BLAKE3 hash string.
///
/// v2 format: `assets.releases[current].arches[arch_key]` -> map of name -> {hash, size}
pub fn extract_hashes(
    manifest: &serde_json::Value,
    _version: &str,
    arch_key: &str,
) -> HashMap<String, String> {
    let mut hashes = HashMap::new();

    // v2 format: assets.releases[current].arches[arch_key]
    if let Some(assets_section) = manifest.get("assets") {
        let current = assets_section
            .get("current")
            .and_then(|c| c.as_str())
            .unwrap_or("");
        if let Some(release) = assets_section.get("releases").and_then(|r| r.get(current)) {
            if let Some(arch_assets) = release.get("arches").and_then(|a| a.get(arch_key)) {
                if let Some(obj) = arch_assets.as_object() {
                    for (name, entry) in obj {
                        if let Some(hash) = entry.get("hash").and_then(|h| h.as_str()) {
                            hashes.insert(name.clone(), hash.to_string());
                        }
                    }
                }
            }
        }
    }

    hashes
}

/// Map a Rust target architecture string to the manifest arch key.
pub fn target_arch_to_key(target_arch: &str) -> &str {
    match target_arch {
        "aarch64" => "arm64",
        "x86_64" => "x86_64",
        _ => "arm64",
    }
}

/// Helper for serializing SystemTime to RFC 3339 strings in JSON.
pub mod time_format {
    use serde::{self, Deserialize, Deserializer, Serializer};
    use std::time::SystemTime;

    pub fn serialize<S>(time: &SystemTime, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let iso = crate::session::epoch_to_iso(
            time.duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        );
        serializer.serialize_str(&iso)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<SystemTime, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        // Parse ISO 8601 subset: YYYY-MM-DDTHH:MM:SS (with optional trailing Z or offset)
        if s.len() >= 19 && s.as_bytes()[10] == b'T' {
            let bad =
                |field: &str| serde::de::Error::custom(format!("invalid {field} in datetime: {s}"));
            let year = s[0..4].parse::<u64>().map_err(|_| bad("year"))?;
            let month = s[5..7].parse::<u64>().map_err(|_| bad("month"))?;
            let day = s[8..10].parse::<u64>().map_err(|_| bad("day"))?;
            let hour = s[11..13].parse::<u64>().map_err(|_| bad("hour"))?;
            let min = s[14..16].parse::<u64>().map_err(|_| bad("minute"))?;
            let sec = s[17..19].parse::<u64>().map_err(|_| bad("second"))?;

            let mut days = 0u64;
            for y in 1970..year {
                days += if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) {
                    366
                } else {
                    365
                };
            }
            let days_in_month = [0, 31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
            let is_leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
            for m in 1..month {
                days += days_in_month[m as usize];
                if m == 2 && is_leap {
                    days += 1;
                }
            }
            days += day - 1;

            let secs = days * 86400 + hour * 3600 + min * 60 + sec;
            Ok(std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs))
        } else {
            Err(serde::de::Error::custom(format!(
                "unsupported datetime format: {s}"
            )))
        }
    }
}

#[cfg(test)]
mod tests;
