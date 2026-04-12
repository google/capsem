//! Manifest hash extraction logic shared between build.rs and runtime.
//!
//! The asset manifest (`manifest.json`) can use two layouts:
//!
//! **Per-arch** (produced by `capsem-builder build`):
//! ```json
//! {"releases": {"0.13.0": {"arm64": {"assets": [{"filename": "vmlinuz", ...}]}}}}
//! ```
//!
//! **Flat** (legacy, bare filenames):
//! ```json
//! {"releases": {"0.13.0": {"assets": [{"filename": "vmlinuz", ...}]}}}
//! ```
//!
//! `extract_hashes` handles both formats, trying per-arch first for the given
//! architecture key, then falling back to the flat layout.

use std::collections::HashMap;

/// Extract asset hashes from a manifest JSON Value for a given version and architecture.
///
/// Returns a map of filename to BLAKE3 hash string.
///
/// Tries per-arch format first: `releases[version][arch_key].assets`
/// Falls back to flat format: `releases[version].assets`
pub fn extract_hashes(
    manifest: &serde_json::Value,
    version: &str,
    arch_key: &str,
) -> HashMap<String, String> {
    let mut hashes = HashMap::new();
    let release = match manifest.get("releases").and_then(|r| r.get(version)) {
        Some(r) => r,
        None => return hashes,
    };

    // Per-arch: releases -> version -> arch_key -> assets
    let assets_value = release
        .get(arch_key)
        .and_then(|a| a.get("assets"))
        // Flat: releases -> version -> assets
        .or_else(|| release.get("assets"));

    if let Some(assets) = assets_value.and_then(|a| a.as_array()) {
        for asset in assets {
            let filename = asset
                .get("filename")
                .and_then(|f| f.as_str())
                .unwrap_or("");
            let hash = asset
                .get("hash")
                .and_then(|h| h.as_str())
                .unwrap_or("");
            if !filename.is_empty() && !hash.is_empty() {
                hashes.insert(filename.to_string(), hash.to_string());
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

    const PER_ARCH_MANIFEST: &str = r#"{
        "latest": "0.13.0",
        "releases": {
            "0.13.0": {
                "arm64": {
                    "assets": [
                        {"filename": "vmlinuz", "hash": "aaa111", "size": 100},
                        {"filename": "initrd.img", "hash": "bbb222", "size": 200},
                        {"filename": "rootfs.squashfs", "hash": "ccc333", "size": 300}
                    ]
                },
                "x86_64": {
                    "assets": [
                        {"filename": "vmlinuz", "hash": "ddd444", "size": 100},
                        {"filename": "initrd.img", "hash": "eee555", "size": 200},
                        {"filename": "rootfs.squashfs", "hash": "fff666", "size": 300}
                    ]
                }
            }
        }
    }"#;

    const FLAT_MANIFEST: &str = r#"{
        "latest": "0.13.0",
        "releases": {
            "0.13.0": {
                "assets": [
                    {"filename": "vmlinuz", "hash": "aaa111", "size": 100},
                    {"filename": "initrd.img", "hash": "bbb222", "size": 200},
                    {"filename": "rootfs.squashfs", "hash": "ccc333", "size": 300}
                ]
            }
        }
    }"#;

    const FLAT_ARCH_PREFIX_MANIFEST: &str = r#"{
        "latest": "0.13.0",
        "releases": {
            "0.13.0": {
                "assets": [
                    {"filename": "arm64/vmlinuz", "hash": "aaa111", "size": 100},
                    {"filename": "arm64/initrd.img", "hash": "bbb222", "size": 200},
                    {"filename": "arm64/rootfs.squashfs", "hash": "ccc333", "size": 300}
                ]
            }
        }
    }"#;

    #[test]
    fn per_arch_arm64_extracts_correct_hashes() {
        let v: serde_json::Value = serde_json::from_str(PER_ARCH_MANIFEST).unwrap();
        let hashes = extract_hashes(&v, "0.13.0", "arm64");
        assert_eq!(hashes.get("vmlinuz").unwrap(), "aaa111");
        assert_eq!(hashes.get("initrd.img").unwrap(), "bbb222");
        assert_eq!(hashes.get("rootfs.squashfs").unwrap(), "ccc333");
    }

    #[test]
    fn per_arch_x86_64_extracts_correct_hashes() {
        let v: serde_json::Value = serde_json::from_str(PER_ARCH_MANIFEST).unwrap();
        let hashes = extract_hashes(&v, "0.13.0", "x86_64");
        assert_eq!(hashes.get("vmlinuz").unwrap(), "ddd444");
        assert_eq!(hashes.get("initrd.img").unwrap(), "eee555");
        assert_eq!(hashes.get("rootfs.squashfs").unwrap(), "fff666");
    }

    #[test]
    fn per_arch_isolates_hashes_between_architectures() {
        let v: serde_json::Value = serde_json::from_str(PER_ARCH_MANIFEST).unwrap();
        let arm64 = extract_hashes(&v, "0.13.0", "arm64");
        let x86 = extract_hashes(&v, "0.13.0", "x86_64");
        assert_ne!(arm64.get("vmlinuz"), x86.get("vmlinuz"));
    }

    #[test]
    fn flat_manifest_extracts_hashes() {
        let v: serde_json::Value = serde_json::from_str(FLAT_MANIFEST).unwrap();
        let hashes = extract_hashes(&v, "0.13.0", "arm64");
        assert_eq!(hashes.get("vmlinuz").unwrap(), "aaa111");
        assert_eq!(hashes.get("initrd.img").unwrap(), "bbb222");
        assert_eq!(hashes.get("rootfs.squashfs").unwrap(), "ccc333");
    }

    #[test]
    fn flat_arch_prefix_filenames_not_found_as_bare_names() {
        // Documents Bug 1: gen_manifest.py produces "arm64/vmlinuz" but
        // build.rs matches on bare "vmlinuz". Hashes are silently missing.
        let v: serde_json::Value = serde_json::from_str(FLAT_ARCH_PREFIX_MANIFEST).unwrap();
        let hashes = extract_hashes(&v, "0.13.0", "arm64");
        // "vmlinuz" is NOT a key -- only "arm64/vmlinuz" is
        assert!(
            !hashes.contains_key("vmlinuz"),
            "bare 'vmlinuz' should not match arch-prefixed 'arm64/vmlinuz'"
        );
        assert_eq!(hashes.get("arm64/vmlinuz").unwrap(), "aaa111");
    }

    #[test]
    fn missing_version_returns_empty() {
        let v: serde_json::Value = serde_json::from_str(PER_ARCH_MANIFEST).unwrap();
        let hashes = extract_hashes(&v, "99.99.99", "arm64");
        assert!(hashes.is_empty());
    }

    #[test]
    fn missing_arch_falls_back_to_flat() {
        let v: serde_json::Value = serde_json::from_str(FLAT_MANIFEST).unwrap();
        let hashes = extract_hashes(&v, "0.13.0", "riscv64");
        // No riscv64 key, falls back to flat assets
        assert_eq!(hashes.get("vmlinuz").unwrap(), "aaa111");
    }

    #[test]
    fn target_arch_mapping() {
        assert_eq!(target_arch_to_key("aarch64"), "arm64");
        assert_eq!(target_arch_to_key("x86_64"), "x86_64");
        assert_eq!(target_arch_to_key("riscv64"), "arm64"); // fallback
    }

    // -- golden fixture tests --

    #[test]
    fn golden_per_arch_fixture_all_hashes_found() {
        let content = include_str!("../../../data/fixtures/manifest_per_arch.json");
        let v: serde_json::Value = serde_json::from_str(content).unwrap();
        for arch in ["arm64", "x86_64"] {
            let hashes = extract_hashes(&v, "0.13.0", arch);
            assert!(hashes.contains_key("vmlinuz"), "vmlinuz missing for {arch}");
            assert!(
                hashes.contains_key("initrd.img"),
                "initrd.img missing for {arch}"
            );
            assert!(
                hashes.contains_key("rootfs.squashfs"),
                "rootfs.squashfs missing for {arch}"
            );
        }
    }

    #[test]
    fn golden_per_arch_fixture_arch_isolation() {
        let content = include_str!("../../../data/fixtures/manifest_per_arch.json");
        let v: serde_json::Value = serde_json::from_str(content).unwrap();
        let arm64 = extract_hashes(&v, "0.13.0", "arm64");
        let x86 = extract_hashes(&v, "0.13.0", "x86_64");
        assert_ne!(
            arm64.get("vmlinuz"),
            x86.get("vmlinuz"),
            "arm64 and x86_64 must have different kernel hashes"
        );
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
