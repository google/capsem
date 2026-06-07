fn main() {
    if let Ok(ts) = std::env::var("CAPSEM_BUILD_TS") {
        println!("cargo:rustc-env=CAPSEM_BUILD_TS={ts}");
    }
    println!("cargo:rerun-if-env-changed=CAPSEM_BUILD_TS");
    tauri_build::build();
}
