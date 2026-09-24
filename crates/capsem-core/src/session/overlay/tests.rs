use std::os::unix::fs::symlink;

use super::*;

/// A session in the layout every VM had before the overlay left the share:
/// the image inside `guest/system/`, reached from the root by a compat link.
fn legacy_session(image: &[u8]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("guest/system")).unwrap();
    std::fs::write(dir.path().join("guest/system/rootfs.img"), image).unwrap();
    symlink("guest/system", dir.path().join("system")).unwrap();
    dir
}

#[test]
fn a_legacy_image_moves_out_of_the_guest_share() {
    let session = legacy_session(b"overlay");

    adopt_system_overlay(session.path()).unwrap();

    let system = session.path().join("system");
    assert!(
        system.is_dir() && !system.is_symlink(),
        "system/ is a real host directory"
    );
    assert_eq!(std::fs::read(system.join("rootfs.img")).unwrap(), b"overlay");
    assert!(!session.path().join("guest/system/rootfs.img").exists());
    assert_eq!(system_overlay_metadata(session.path()).unwrap().len(), 7);
}

#[test]
fn adoption_is_idempotent_and_keeps_the_host_image() {
    let session = legacy_session(b"overlay");
    adopt_system_overlay(session.path()).unwrap();
    // Anything the guest writes into its old location afterwards is ignored.
    std::fs::write(session.path().join("guest/system/rootfs.img"), b"guest").unwrap();

    adopt_system_overlay(session.path()).unwrap();

    assert_eq!(
        std::fs::read(system_overlay_image_path(session.path())).unwrap(),
        b"overlay"
    );
}

#[test]
fn a_guest_planted_link_is_refused_never_attached() {
    let host = tempfile::tempdir().unwrap();
    let secret = host.path().join("id_ed25519");
    std::fs::write(&secret, b"host secret").unwrap();
    let session = legacy_session(b"");
    let planted = session.path().join("guest/system/rootfs.img");
    std::fs::remove_file(&planted).unwrap();
    symlink(&secret, &planted).unwrap();

    let error = adopt_system_overlay(session.path()).unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied, "{error}");
    assert!(
        std::fs::symlink_metadata(system_overlay_image_path(session.path())).is_err(),
        "the link is not left where the host would attach it"
    );
    assert_eq!(std::fs::read(&secret).unwrap(), b"host secret");
    assert!(system_overlay_metadata(session.path()).is_err());
}

#[test]
fn a_guest_linked_system_directory_is_refused() {
    let host = tempfile::tempdir().unwrap();
    std::fs::write(host.path().join("rootfs.img"), b"host file").unwrap();
    let session = legacy_session(b"");
    std::fs::remove_dir_all(session.path().join("guest/system")).unwrap();
    symlink(host.path(), session.path().join("guest/system")).unwrap();

    assert!(adopt_system_overlay(session.path()).is_err());
    assert_eq!(std::fs::read(host.path().join("rootfs.img")).unwrap(), b"host file");
    assert!(std::fs::symlink_metadata(system_overlay_image_path(session.path())).is_err());
}

#[test]
fn readers_refuse_a_link_at_every_level() {
    let host = tempfile::tempdir().unwrap();
    std::fs::write(host.path().join("rootfs.img"), b"host file").unwrap();
    let linked_dir = legacy_session(b"overlay");
    let linked_image = tempfile::tempdir().unwrap();
    std::fs::create_dir(linked_image.path().join("system")).unwrap();
    symlink(
        host.path().join("rootfs.img"),
        linked_image.path().join("system/rootfs.img"),
    )
    .unwrap();

    // An unmigrated session is reached through its compat link: not read.
    assert!(open_system_overlay(linked_dir.path()).is_err());
    assert!(open_system_overlay(linked_image.path()).is_err());
}

#[test]
fn a_fresh_session_gets_an_empty_host_directory() {
    let session = tempfile::tempdir().unwrap();

    adopt_system_overlay(session.path()).unwrap();

    assert!(session.path().join("system").is_dir());
    assert!(!system_overlay_image_path(session.path()).exists());
}
