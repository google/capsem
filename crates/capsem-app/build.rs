use std::fs;
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=../../assets/manifest.json");
    println!("cargo:rerun-if-changed=../../assets/B3SUMS");

    // Extract hashes from manifest.json (preferred) or B3SUMS (legacy fallback).
    // Optional: only set env vars if the file exists (so developers can still
    // build without running build.py, though boot will fail at runtime).
    let manifest_path = Path::new("../../assets/manifest.json");
    let b3sums_path = Path::new("../../assets/B3SUMS");

    if manifest_path.exists() {
        // Parse manifest.json to extract hashes for the current version.
        let content = fs::read_to_string(manifest_path).unwrap();
        if let Ok(manifest) = serde_json::from_str::<serde_json::Value>(&content) {
            let version = env!("CARGO_PKG_VERSION");
            if let Some(release) = manifest.get("releases").and_then(|r| r.get(version)) {
                if let Some(assets) = release.get("assets").and_then(|a| a.as_array()) {
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
