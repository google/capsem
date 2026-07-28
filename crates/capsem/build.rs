use std::path::Path;

fn git_output(args: &[&str]) -> Option<String> {
    let output = std::process::Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!value.is_empty()).then_some(value)
}

fn watch_git_path(pathspec: &str) {
    if let Some(path) = git_output(&["rev-parse", "--git-path", pathspec]) {
        if Path::new(&path).exists() {
            println!("cargo:rerun-if-changed={path}");
        }
    }
}

fn main() {
    // Embed a unique build hash: git short SHA + build timestamp.
    // Changes on every recompile, even from the same commit.
    let git_hash =
        git_output(&["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| "unknown".to_string());
    let build_ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    println!("cargo:rustc-env=CAPSEM_BUILD_HASH={git_hash}.{build_ts}");
    if let Ok(ts) = std::env::var("CAPSEM_BUILD_TS") {
        println!("cargo:rustc-env=CAPSEM_BUILD_TS={ts}");
    }
    println!("cargo:rerun-if-env-changed=CAPSEM_BUILD_TS");

    // `.git/HEAD` normally contains only `ref: refs/heads/<branch>`, so it does
    // not change when that branch advances. Resolve Git's real metadata paths
    // to support ordinary repositories, detached CI checkouts, and worktrees.
    watch_git_path("HEAD");
    watch_git_path("logs/HEAD");
    watch_git_path("packed-refs");
    if let Some(head_ref) = git_output(&["symbolic-ref", "-q", "HEAD"]) {
        watch_git_path(&head_ref);
    }

    // Explicit rerun directives disable Cargo's package-wide default, so keep
    // source edits and package metadata in the build identity as well.
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=Cargo.toml");
}
