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
        let current = assets_section.get("current").and_then(|c| c.as_str()).unwrap_or("");
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
            let bad = |field: &str| serde::de::Error::custom(format!("invalid {field} in datetime: {s}"));
            let year = s[0..4].parse::<u64>().map_err(|_| bad("year"))?;
            let month = s[5..7].parse::<u64>().map_err(|_| bad("month"))?;
            let day = s[8..10].parse::<u64>().map_err(|_| bad("day"))?;
            let hour = s[11..13].parse::<u64>().map_err(|_| bad("hour"))?;
            let min = s[14..16].parse::<u64>().map_err(|_| bad("minute"))?;
            let sec = s[17..19].parse::<u64>().map_err(|_| bad("second"))?;

            let mut days = 0u64;
            for y in 1970..year {
                days += if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) { 366 } else { 365 };
            }
            let days_in_month = [0, 31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
            let is_leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
            for m in 1..month {
                days += days_in_month[m as usize];
                if m == 2 && is_leap { days += 1; }
            }
            days += day - 1;

            let secs = days * 86400 + hour * 3600 + min * 60 + sec;
            Ok(std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs))
        } else {
            Err(serde::de::Error::custom(format!("unsupported datetime format: {s}")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const V2_MANIFEST: &str = r#"{
        "format": 2,
        "assets": {
            "current": "2026.0415.1",
            "releases": {
                "2026.0415.1": {
                    "date": "2026-04-15",
                    "deprecated": false,
                    "min_binary": "1.0.0",
                    "arches": {
                        "arm64": {
                            "vmlinuz": { "hash": "aaa111", "size": 100 },
                            "initrd.img": { "hash": "bbb222", "size": 200 },
                            "rootfs.squashfs": { "hash": "ccc333", "size": 300 }
                        },
                        "x86_64": {
                            "vmlinuz": { "hash": "ddd444", "size": 100 },
                            "initrd.img": { "hash": "eee555", "size": 200 },
                            "rootfs.squashfs": { "hash": "fff666", "size": 300 }
                        }
                    }
                }
            }
        },
        "binaries": {
            "current": "1.0.1000000000",
            "releases": {
                "1.0.1000000000": {
                    "date": "2026-04-15",
                    "deprecated": false,
                    "min_assets": "2026.0415.1"
                }
            }
        }
    }"#;

    #[test]
    fn v2_arm64_extracts_correct_hashes() {
        let v: serde_json::Value = serde_json::from_str(V2_MANIFEST).unwrap();
        let hashes = extract_hashes(&v, "", "arm64");
        assert_eq!(hashes.get("vmlinuz").unwrap(), "aaa111");
        assert_eq!(hashes.get("initrd.img").unwrap(), "bbb222");
        assert_eq!(hashes.get("rootfs.squashfs").unwrap(), "ccc333");
    }

    #[test]
    fn v2_x86_64_extracts_correct_hashes() {
        let v: serde_json::Value = serde_json::from_str(V2_MANIFEST).unwrap();
        let hashes = extract_hashes(&v, "", "x86_64");
        assert_eq!(hashes.get("vmlinuz").unwrap(), "ddd444");
        assert_eq!(hashes.get("initrd.img").unwrap(), "eee555");
        assert_eq!(hashes.get("rootfs.squashfs").unwrap(), "fff666");
    }

    #[test]
    fn v2_arch_isolation() {
        let v: serde_json::Value = serde_json::from_str(V2_MANIFEST).unwrap();
        let arm64 = extract_hashes(&v, "", "arm64");
        let x86 = extract_hashes(&v, "", "x86_64");
        assert_ne!(arm64.get("vmlinuz"), x86.get("vmlinuz"));
    }

    #[test]
    fn missing_arch_returns_empty() {
        let v: serde_json::Value = serde_json::from_str(V2_MANIFEST).unwrap();
        let hashes = extract_hashes(&v, "", "riscv64");
        assert!(hashes.is_empty());
    }

    #[test]
    fn target_arch_mapping() {
        assert_eq!(target_arch_to_key("aarch64"), "arm64");
        assert_eq!(target_arch_to_key("x86_64"), "x86_64");
        assert_eq!(target_arch_to_key("riscv64"), "arm64");
    }

    // -- time_format serde tests --

    #[derive(serde::Serialize, serde::Deserialize, Debug)]
    struct TimeWrapper {
        #[serde(with = "time_format")]
        t: std::time::SystemTime,
    }

    #[test]
    fn time_format_roundtrip() {
        let now = std::time::SystemTime::now();
        let w = TimeWrapper { t: now };
        let json = serde_json::to_string(&w).unwrap();
        let w2: TimeWrapper = serde_json::from_str(&json).unwrap();
        // Allow 1s tolerance (sub-second precision is lost)
        let diff = now.duration_since(w2.t).unwrap_or_default();
        assert!(diff.as_secs() <= 1, "roundtrip drift too large: {:?}", diff);
    }

    #[test]
    fn time_format_rejects_garbage() {
        let json = r#"{"t":"not-a-date"}"#;
        let result = serde_json::from_str::<TimeWrapper>(json);
        assert!(result.is_err(), "garbage timestamp should fail, not silently return epoch");
    }

    #[test]
    fn time_format_rejects_empty() {
        let json = r#"{"t":""}"#;
        let result = serde_json::from_str::<TimeWrapper>(json);
        assert!(result.is_err(), "empty timestamp should fail, not silently return epoch");
    }
}
