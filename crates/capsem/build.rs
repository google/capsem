fn main() {
    let git_head = std::path::Path::new("../../.git/HEAD");

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
    if let Ok(ts) = std::env::var("CAPSEM_BUILD_TS") {
        println!("cargo:rustc-env=CAPSEM_BUILD_TS={ts}");
    }

    // Rebuild when HEAD moves. On a normal branch, .git/HEAD contains a
    // symbolic ref and does not change for each commit; the branch ref does.
    println!("cargo:rerun-if-changed={}", git_head.display());
    println!("cargo:rerun-if-changed=../../.git/packed-refs");
    if let Ok(head) = std::fs::read_to_string(git_head) {
        if let Some(reference) = head.strip_prefix("ref: ").map(str::trim) {
            println!("cargo:rerun-if-changed=../../.git/{reference}");
        }
    }
}
