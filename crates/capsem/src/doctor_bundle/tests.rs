use super::*;

/// The doctor bundle is guest-written: a link planted in its place must not
/// copy a host file into the bundle the user is about to share.
#[test]
fn doctor_bundle_copy_refuses_a_guest_planted_link() {
    let dir = tempfile::tempdir().unwrap();
    let session = dir.path().join("vm");
    std::fs::create_dir_all(session.join("guest/workspace")).unwrap();
    let secret = dir.path().join("id_ed25519");
    std::fs::write(&secret, b"host secret").unwrap();
    std::os::unix::fs::symlink(&secret, session.join("guest/doctor-bundle.tar")).unwrap();
    let dest = dir.path().join("doctor-latest.tar");

    assert!(copy_out(&session, &dest).is_err());
    assert!(!dest.exists(), "nothing was copied from the host");

    std::fs::remove_file(session.join("guest/doctor-bundle.tar")).unwrap();
    std::fs::write(session.join("guest/workspace/doctor-bundle.tar"), b"tar").unwrap();
    assert_eq!(copy_out(&session, &dest).unwrap(), 3);
    assert_eq!(std::fs::read(&dest).unwrap(), b"tar");
}
