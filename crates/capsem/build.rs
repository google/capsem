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
    // The revision, told to us if the builder knows it, discovered otherwise.
    //
    // A container built from `COPY . /src` has no `.git` -- it is excluded by
    // `.dockerignore`, and shipping a 100 MB repository into every lane image
    // to answer one question would be the wrong trade. So the gate passes the
    // revision it already recorded, and git is the fallback for an ordinary
    // developer build.
    //
    // Falling back to "unknown" silently is deliberate here and checked
    // elsewhere: `scripts/check-build-provenance.sh` refuses a binary whose
    // embedded revision is not the expected one, so an unset variable fails
    // loudly at that gate rather than shipping a mislabelled package.
    let git_hash = std::env::var("CAPSEM_BUILD_REVISION")
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(|| git_output(&["rev-parse", "--short", "HEAD"]))
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rerun-if-env-changed=CAPSEM_BUILD_REVISION");
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
