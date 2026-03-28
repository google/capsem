use std::fs;
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=../../assets/manifest.json");
    println!("cargo:rerun-if-changed=../../assets/B3SUMS");

    // Extract hashes from manifest.json (preferred) or B3SUMS (legacy fallback).
    // This logic mirrors capsem_core::manifest_compat::extract_hashes() --
    // keep them in sync. Tests in manifest_compat validate the algorithm.
    let manifest_path = Path::new("../../assets/manifest.json");
    let b3sums_path = Path::new("../../assets/B3SUMS");

    if manifest_path.exists() {
        // Parse manifest.json to extract hashes for the current version.
        let content = fs::read_to_string(manifest_path).unwrap();
        if let Ok(manifest) = serde_json::from_str::<serde_json::Value>(&content) {
            let version = env!("CARGO_PKG_VERSION");
            if let Some(release) = manifest.get("releases").and_then(|r| r.get(version)) {
                // Determine target architecture for per-arch manifest lookup.
                let target_arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
                let arch_key = match target_arch.as_str() {
                    "aarch64" => "arm64",
                    "x86_64" => "x86_64",
                    _ => "arm64",
                };

                // Try per-arch format first: releases -> version -> arch -> assets
                let assets_value = release
                    .get(arch_key)
                    .and_then(|a| a.get("assets"))
                    // Fall back to flat format: releases -> version -> assets
                    .or_else(|| release.get("assets"));

                if let Some(assets) = assets_value.and_then(|a| a.as_array()) {
                    for asset in assets {
                        let filename = asset.get("filename").and_then(|f| f.as_str()).unwrap_or("");
                        let hash = asset.get("hash").and_then(|h| h.as_str()).unwrap_or("");
                        match filename {
                            "vmlinuz" => println!("cargo:rustc-env=VMLINUZ_HASH={}", hash),
                            "initrd.img" => println!("cargo:rustc-env=INITRD_HASH={}", hash),
                            "rootfs.squashfs" => {
                                println!("cargo:rustc-env=ROOTFS_HASH={}", hash);
                                println!("cargo:rustc-env=ROOTFS_FILENAME={}", filename);
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    } else if b3sums_path.exists() {
        // Legacy B3SUMS fallback.
        let content = fs::read_to_string(b3sums_path).unwrap();
        for line in content.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() == 2 {
                let hash = parts[0];
                let filename = parts[1];
                match filename {
                    "vmlinuz" => println!("cargo:rustc-env=VMLINUZ_HASH={}", hash),
                    "initrd.img" => println!("cargo:rustc-env=INITRD_HASH={}", hash),
                    "rootfs.squashfs" => {
                        println!("cargo:rustc-env=ROOTFS_HASH={}", hash);
                        println!("cargo:rustc-env=ROOTFS_FILENAME={}", filename);
                    }
                    _ => {}
                }
            }
        }
    }

    tauri_build::build()
}
