//! Tests for `manifest_compat` (extracted from inline `mod tests`).

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
    assert!(
        result.is_err(),
        "garbage timestamp should fail, not silently return epoch"
    );
}

#[test]
fn time_format_rejects_empty() {
    let json = r#"{"t":""}"#;
    let result = serde_json::from_str::<TimeWrapper>(json);
    assert!(
        result.is_err(),
        "empty timestamp should fail, not silently return epoch"
    );
}
