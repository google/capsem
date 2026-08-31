//! Linking is the only operation that makes two paths the same file.

use super::*;
use std::fs;

fn checkout(root: &std::path::Path) -> std::path::PathBuf {
    let source = root.join("config/profiles/code");
    fs::create_dir_all(&source).unwrap();
    let seed = source.join("root.manifest.json");
    fs::write(&seed, b"{}").unwrap();
    seed
}

#[test]
fn staging_checked_in_source_into_output_copies_instead_of_linking() {
    // The defect, in one assertion. `capsem-admin` hardlinked profile seeds
    // into the published release channel, so 48 tracked files sat inside
    // build output sharing an inode. A `chmod` on the artifact rewrote the
    // source file, and no content digest could notice: the bytes never moved.
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let seed = checkout(root);
    let published = root.join("target/distribution/root.manifest.json");

    stage(&seed, &published, root).unwrap();

    assert_eq!(fs::read(&published).unwrap(), b"{}");
    assert_ne!(
        fs::metadata(&seed).unwrap().ino(),
        fs::metadata(&published).unwrap().ino(),
        "the published artifact is the same file as checked-in source"
    );
    assert_eq!(fs::metadata(&seed).unwrap().nlink(), 1, "the source file gained a link");
}

#[test]
fn a_chmod_on_the_published_artifact_cannot_reach_the_source() {
    // Why the inode matters, stated as the consequence rather than the
    // mechanism. This is the failure a reader has to be able to picture.
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let seed = checkout(root);
    let published = root.join("target/distribution/root.manifest.json");
    stage(&seed, &published, root).unwrap();

    fs::set_permissions(&published, fs::Permissions::from_mode(0o000)).unwrap();

    let source_mode = fs::metadata(&seed).unwrap().permissions().mode() & 0o777;
    assert_ne!(source_mode, 0o000, "chmod on the artifact reached the source");
    assert!(fs::read(&seed).is_ok(), "checked-in source became unreadable");
}

#[test]
fn staging_build_output_into_build_output_still_hardlinks() {
    // The optimization is real and worth keeping: asset staging moves
    // multi-gigabyte images, and copying them because of a rule aimed at
    // small checked-in seeds would trade one defect for an hour of I/O.
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let built = root.join("target/assets/rootfs.erofs");
    fs::create_dir_all(built.parent().unwrap()).unwrap();
    fs::write(&built, b"image").unwrap();
    let staged = root.join("target/distribution/rootfs.erofs");

    stage(&built, &staged, root).unwrap();

    assert_eq!(
        fs::metadata(&built).unwrap().ino(),
        fs::metadata(&staged).unwrap().ino(),
        "build output should still be linked, not copied"
    );
}

#[test]
fn a_cross_device_link_falls_back_to_copying() {
    // `EXDEV` is the documented reason `hardlink_or_copy` existed at all, and
    // dropping it would break staging onto a different filesystem.
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let built = root.join("target/assets/x");
    fs::create_dir_all(built.parent().unwrap()).unwrap();
    fs::write(&built, b"bytes").unwrap();

    // A destination whose parent does not exist yet exercises the same path
    // the release staging takes.
    let staged = root.join("target/deep/nested/x");
    stage(&built, &staged, root).unwrap();
    assert_eq!(fs::read(&staged).unwrap(), b"bytes");
}

#[test]
fn a_relative_source_path_is_not_assumed_to_be_build_output() {
    // The first version of `stage` answered "outside the checkout" for
    // anything `strip_prefix` could not resolve, and then hardlinked it. A
    // relative path -- which is what the release scripts actually pass -- took
    // that branch, so the fix shipped and 192 checked-in files were still
    // linked into the published channel on the very next build.
    //
    // Not knowing must mean copying. A needless copy costs I/O; a needless
    // link costs a published artifact that shares an inode with tracked
    // source, which is the defect this module exists for.
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let seed = checkout(root);
    let published = root.join("target/distribution/root.manifest.json");

    let previous = std::env::current_dir().unwrap();
    std::env::set_current_dir(root).unwrap();
    let result = stage(
        std::path::Path::new("config/profiles/code/root.manifest.json"),
        &published,
        root,
    );
    std::env::set_current_dir(previous).unwrap();
    result.unwrap();

    assert_ne!(
        fs::metadata(&seed).unwrap().ino(),
        fs::metadata(&published).unwrap().ino(),
        "a relative checked-in path was treated as build output and linked"
    );
}

#[test]
fn a_source_that_cannot_be_classified_is_copied() {
    // The rule stated directly, so it cannot drift back to failing open.
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let elsewhere = tempfile::tempdir().unwrap();
    let source = elsewhere.path().join("unknown.bin");
    fs::write(&source, b"bytes").unwrap();
    let published = root.join("target/out.bin");

    stage(&source, &published, root).unwrap();

    assert_ne!(
        fs::metadata(&source).unwrap().ino(),
        fs::metadata(&published).unwrap().ino(),
    );
}
