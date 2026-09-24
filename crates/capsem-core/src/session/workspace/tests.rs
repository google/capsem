use std::os::unix::fs::symlink;

use super::*;

#[test]
fn opens_the_workspace_inside_the_share() {
    let session = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(session.path().join("guest/workspace")).unwrap();
    std::fs::write(session.path().join("guest/workspace/a.txt"), b"a").unwrap();

    let workspace = open_workspace(session.path()).unwrap();

    assert_eq!(workspace.entries().unwrap().len(), 1);
}

#[test]
fn a_session_from_before_the_share_keeps_its_root_workspace() {
    let session = tempfile::tempdir().unwrap();
    std::fs::create_dir(session.path().join("workspace")).unwrap();

    assert!(open_workspace(session.path()).is_ok());
}

#[test]
fn a_guest_linked_workspace_or_share_is_refused() {
    let host = tempfile::tempdir().unwrap();
    std::fs::write(host.path().join("secret"), b"host").unwrap();
    let linked_workspace = tempfile::tempdir().unwrap();
    std::fs::create_dir(linked_workspace.path().join("guest")).unwrap();
    symlink(host.path(), linked_workspace.path().join("guest/workspace")).unwrap();
    let linked_share = tempfile::tempdir().unwrap();
    symlink(host.path(), linked_share.path().join("guest")).unwrap();

    for session in [&linked_workspace, &linked_share] {
        let error = open_workspace(session.path()).unwrap_err();
        assert!(
            capsem_foundation::unix::contained::is_symlink_refusal(&error),
            "{error}"
        );
    }
}
