fn main() {
    // Embed a unique build hash: git short SHA + build timestamp.
    // Changes on every recompile, even from the same commit.
    let git_hash = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "unknown".into());
    let build_ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    println!("cargo:rustc-env=CAPSEM_BUILD_HASH={git_hash}.{build_ts}");
    // Rebuild when git HEAD changes or any source changes
    println!("cargo:rerun-if-changed=../../.git/HEAD");
}
