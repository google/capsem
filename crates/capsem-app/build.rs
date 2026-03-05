use std::fs;
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=../../assets/B3SUMS");

    // Optional: Only enforce signature at build time if the file exists
    // (so developers can still build if they haven't run build.py yet,
    // though boot will fail if we enforce it at runtime).
    let manifest_path = Path::new("../../assets/B3SUMS");
    if manifest_path.exists() {
        let content = fs::read_to_string(manifest_path).unwrap();
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
