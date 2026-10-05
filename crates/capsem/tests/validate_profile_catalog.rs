//! The new-binary half of #288, through the real `capsem` executable.
//!
//! An updater that installs a newer binary hands it the staged profile
//! catalog as `capsem update --validate-profile-catalog <dir>` and activates
//! only on exit status 0. These tests hold that contract on the binary itself:
//! a catalog it parses exits 0, one carrying a field it does not know exits
//! non-zero and names the field, and the flag cannot ride along with an update.

use std::path::Path;
use std::process::{Command, Output};

const CODE_PROFILE: &str = include_str!("../../../config/profiles/code/profile.toml");

fn validate(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_capsem"))
        .arg("update")
        .args(args)
        .env("HOME", home)
        .env("CAPSEM_HOME", home.join(".capsem"))
        .env_remove("CAPSEM_PROFILES_DIR")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("run capsem")
}

fn catalog(root: &Path, profile_toml: &str) -> std::path::PathBuf {
    let dir = root.join("profiles");
    std::fs::create_dir_all(dir.join("code")).unwrap();
    std::fs::write(dir.join("code/profile.toml"), profile_toml).unwrap();
    dir
}

#[test]
fn the_binary_accepts_a_catalog_it_parses() {
    let temp = tempfile::tempdir().unwrap();
    let dir = catalog(temp.path(), CODE_PROFILE);

    let output = validate(temp.path(), &["--validate-profile-catalog", dir.to_str().unwrap()]);

    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
}

#[test]
fn the_binary_refuses_a_field_it_does_not_know_and_names_it() {
    let temp = tempfile::tempdir().unwrap();
    let dir = catalog(
        temp.path(),
        &format!("field_from_a_later_release = true\n{CODE_PROFILE}"),
    );

    let output = validate(temp.path(), &["--validate-profile-catalog", dir.to_str().unwrap()]);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("field_from_a_later_release"), "{stderr}");
}

#[test]
fn the_binary_refuses_a_directory_with_no_catalog() {
    let temp = tempfile::tempdir().unwrap();

    let output = validate(
        temp.path(),
        &[
            "--validate-profile-catalog",
            temp.path().join("absent").to_str().unwrap(),
        ],
    );

    assert!(!output.status.success());
}

#[test]
fn the_validation_flag_cannot_ride_along_with_an_update() {
    let temp = tempfile::tempdir().unwrap();
    let dir = catalog(temp.path(), CODE_PROFILE);

    let output = validate(
        temp.path(),
        &["--yes", "--validate-profile-catalog", dir.to_str().unwrap()],
    );

    assert_eq!(output.status.code(), Some(2), "a usage error, before any update runs");
}
