use std::os::unix::fs::{MetadataExt, PermissionsExt};

use super::*;

const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const C: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

/// A session directory and a verified pull's `blobs/sha256` beside it.
fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let session = dir.path().join("session");
    let pull = dir.path().join("pull");
    std::fs::create_dir_all(&session).unwrap();
    std::fs::create_dir_all(&pull).unwrap();
    for name in [A, B, C] {
        std::fs::write(pull.join(name), name.as_bytes()).unwrap();
    }
    (dir, session, pull)
}

fn open(path: &Path) -> ContainedDir {
    ContainedDir::open_root(path).unwrap()
}

#[test]
fn a_published_share_holds_exactly_the_image_blobs_linked_and_read_only() {
    let (_dir, session, pull) = fixture();
    let path = prepare_image_share(&session).unwrap();
    assert_eq!(path, session.join(IMAGE_SHARE_DIR));
    assert!(image_share_blobs(&session).unwrap().is_empty());

    publish_image_share(&session, &open(&pull), &[B.into(), A.into(), A.into()]).unwrap();
    assert_eq!(image_share_blobs(&session).unwrap(), [A, B]);
    for name in [A, B] {
        let shared = std::fs::metadata(path.join(name)).unwrap();
        // A link: the pull's verified bytes, no copy.
        assert_eq!(shared.ino(), std::fs::metadata(pull.join(name)).unwrap().ino());
        assert_eq!(shared.permissions().mode() & 0o777, 0o444);
    }
    assert!(!path.join(C).exists(), "a blob the image does not name was shared");
}

#[test]
fn publishing_again_leaves_no_blob_of_the_previous_image() {
    let (_dir, session, pull) = fixture();
    publish_image_share(&session, &open(&pull), &[A.into(), B.into()]).unwrap();
    publish_image_share(&session, &open(&pull), &[C.into()]).unwrap();
    assert_eq!(image_share_blobs(&session).unwrap(), [C]);
    clear_image_share(&session).unwrap();
    assert!(image_share_blobs(&session).unwrap().is_empty());
}

#[test]
fn only_sha256_names_of_regular_files_are_published() {
    let (dir, session, pull) = fixture();
    std::fs::write(dir.path().join("host-secret"), b"secret").unwrap();
    let link = "d".repeat(64);
    std::os::unix::fs::symlink(dir.path().join("host-secret"), pull.join(&link)).unwrap();
    for bad in ["index.json", "../aaaa", &"A".repeat(64), &"a".repeat(63), &link] {
        assert!(
            publish_image_share(&session, &open(&pull), &[bad.to_string()]).is_err(),
            "{bad} was published"
        );
        assert!(image_share_blobs(&session).unwrap().is_empty(), "{bad} left an entry");
    }
}

#[test]
fn a_clone_carries_its_source_share_by_link() {
    let (dir, session, pull) = fixture();
    publish_image_share(&session, &open(&pull), &[A.into(), B.into()]).unwrap();
    let clone = dir.path().join("clone");
    std::fs::create_dir(&clone).unwrap();
    carry_image_share(&session, &clone).unwrap();
    assert_eq!(image_share_blobs(&clone).unwrap(), [A, B]);
    assert_eq!(
        std::fs::metadata(clone.join(IMAGE_SHARE_DIR).join(A)).unwrap().ino(),
        std::fs::metadata(session.join(IMAGE_SHARE_DIR).join(A)).unwrap().ino()
    );

    // A source that never staged an image gives the clone an empty share,
    // whatever the destination held.
    let bare = dir.path().join("bare");
    std::fs::create_dir(&bare).unwrap();
    carry_image_share(&bare, &clone).unwrap();
    assert!(image_share_blobs(&clone).unwrap().is_empty());
}

#[test]
fn trusted_launch_controls_are_carried_without_sharing_mutable_inodes() {
    let (dir, session, pull) = fixture();
    publish_image_share(&session, &open(&pull), &[A.into()]).unwrap();
    publish_launch_control(&session, "options.json", b"original").unwrap();
    publish_launch_control(&session, "launch.py", b"trusted launcher").unwrap();
    let clone = dir.path().join("clone");
    std::fs::create_dir(&clone).unwrap();
    carry_image_share(&session, &clone).unwrap();
    assert_eq!(image_share_blobs(&clone).unwrap(), [A]);
    let copied = clone.join(IMAGE_SHARE_DIR).join("options.json");
    assert_eq!(std::fs::read(&copied).unwrap(), b"original");
    assert_ne!(
        std::fs::metadata(&copied).unwrap().ino(),
        std::fs::metadata(session.join(IMAGE_SHARE_DIR).join("options.json"))
            .unwrap()
            .ino()
    );
    publish_launch_control(&session, "options.json", b"changed").unwrap();
    assert_eq!(std::fs::read(&copied).unwrap(), b"original");
}

#[test]
fn launch_control_publication_refuses_foreign_names_links_and_oversize() {
    let (dir, session, _) = fixture();
    let outside = dir.path().join("outside");
    std::fs::write(&outside, b"keep").unwrap();
    let share = prepare_image_share(&session).unwrap();
    for name in ["options.json", "launch.py"] {
        std::os::unix::fs::symlink(&outside, share.join(name)).unwrap();
        assert!(publish_launch_control(&session, name, b"replace").is_err());
        assert_eq!(std::fs::read(&outside).unwrap(), b"keep");
        std::fs::remove_file(share.join(name)).unwrap();
    }
    for name in ["../outside", "foreign.py", A] {
        assert!(publish_launch_control(&session, name, b"bad").is_err());
    }
    assert!(publish_launch_control(&session, "options.json", &vec![0; LAUNCH_CONTROL_LIMIT as usize + 1]).is_err());
    assert!(image_share_blobs(&session).unwrap().is_empty());
}

#[test]
fn a_share_holding_anything_but_blobs_is_refused() {
    let (dir, session, _pull) = fixture();
    let share = prepare_image_share(&session).unwrap();
    std::fs::create_dir(share.join("nested")).unwrap();
    assert!(image_share_blobs(&session).is_err());
    assert!(clear_image_share(&session).is_err(), "a directory was recursed into");
    std::fs::remove_dir(share.join("nested")).unwrap();
    std::os::unix::fs::symlink(dir.path(), share.join(A)).unwrap();
    assert!(image_share_blobs(&session).is_err());
    // Clearing removes the link itself, never its target.
    clear_image_share(&session).unwrap();
    assert!(dir.path().join("session").exists());
}

#[test]
fn a_link_in_place_of_the_share_is_refused() {
    let (dir, session, pull) = fixture();
    std::os::unix::fs::symlink(dir.path(), session.join(IMAGE_SHARE_DIR)).unwrap();
    assert!(prepare_image_share(&session).is_err());
    assert!(publish_image_share(&session, &open(&pull), &[A.into()]).is_err());
    assert!(
        !dir.path().join(A).exists(),
        "a blob was linked through the planted link"
    );
}
