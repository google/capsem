//! Containment against a hostile guest driver.
//!
//! The documented guarantee is that the guest cannot reach host files outside
//! the share. A benign guest kernel never sends these request shapes (it
//! resolves symlinks itself via READLINK), so every one below is a malicious
//! driver, which is exactly the threat model. The host must never follow a
//! guest-controlled symlink: not as a parent for a namespace operation, not as
//! the target of CREATE or SETATTR, and it must never block on a FIFO.

use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

use super::tests::{
    build_request, lookup, make_header, open_dir, open_file, response_error, temp_share, test_processor,
};
use super::FuseProcessor;
use crate::hypervisor::fuse::{self, *};

struct Attack {
    share: PathBuf,
    outside: PathBuf,
    proc: FuseProcessor,
}

fn attack(name: &str) -> Attack {
    let share = temp_share(&format!("contain-{name}"));
    let outside = temp_share(&format!("contain-{name}-outside"));
    std::fs::write(outside.join("authorized_keys"), b"ssh-ed25519 victim").unwrap();
    std::fs::write(outside.join("id_rsa"), b"PRIVATE KEY").unwrap();
    std::fs::create_dir_all(outside.join("dir")).unwrap();
    let proc = test_processor(&share);
    Attack { share, outside, proc }
}

fn outside_intact(a: &Attack) {
    assert_eq!(
        std::fs::read(a.outside.join("authorized_keys")).unwrap(),
        b"ssh-ed25519 victim"
    );
    assert_eq!(std::fs::read(a.outside.join("id_rsa")).unwrap(), b"PRIVATE KEY");
    assert!(a.outside.join("dir").is_dir());
    let names: Vec<_> = std::fs::read_dir(&a.outside)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names.len(), 3, "nothing may appear outside: {names:?}");
}

fn name_body(name: &str) -> Vec<u8> {
    let mut body = name.as_bytes().to_vec();
    body.push(0);
    body
}

fn two_names(a: &str, b: &str) -> Vec<u8> {
    let mut body = name_body(a);
    body.extend(name_body(b));
    body
}

fn request(proc: &mut FuseProcessor, opcode: u32, nodeid: u64, body: &[u8]) -> i32 {
    let resp = proc.handle_request(&build_request(&make_header(opcode, nodeid, 7), body));
    response_error(&resp)
}

fn create_body(flags: u32, name: &str) -> Vec<u8> {
    let create_in = FuseCreateIn {
        flags,
        mode: 0o644,
        umask: 0,
        open_flags: 0,
    };
    let mut body = fuse::as_bytes(&create_in).to_vec();
    body.extend(name_body(name));
    body
}

fn setattr_body(valid: u32, size: u64, mode: u32, mtime: u64) -> Vec<u8> {
    fuse::as_bytes(&FuseSetAttrIn {
        valid,
        padding: 0,
        fh: 0,
        size,
        lock_owner: 0,
        atime: 0,
        mtime,
        ctime: 0,
        atimensec: 0,
        mtimensec: 0,
        ctimensec: 0,
        mode,
        unused4: 0,
        uid: 0,
        gid: 0,
        unused5: 0,
    })
    .to_vec()
}

/// A symlink to a directory outside the share, looked up as an inode the
/// driver then uses as a parent.
fn dirlink(a: &mut Attack) -> u64 {
    std::os::unix::fs::symlink(&a.outside, a.share.join("dirlink")).unwrap();
    lookup(&mut a.proc, 1, "dirlink").expect("a symlink is a legitimate inode")
}

#[test]
fn unlink_through_a_symlinked_parent_does_not_delete_outside() {
    let mut a = attack("unlink");
    let parent = dirlink(&mut a);
    let err = request(&mut a.proc, FUSE_UNLINK, parent, &name_body("authorized_keys"));
    assert!(err < 0, "unlink through a symlink parent must fail");
    outside_intact(&a);
}

#[test]
fn rename_through_a_symlinked_parent_does_not_steal_outside_files() {
    let mut a = attack("rename");
    let parent = dirlink(&mut a);
    let mut body = fuse::as_bytes(&FuseRenameIn { newdir: 1 }).to_vec();
    body.extend(two_names("id_rsa", "stolen"));
    let err = request(&mut a.proc, FUSE_RENAME, parent, &body);
    assert!(err < 0, "rename out of a symlink parent must fail");
    assert!(!a.share.join("stolen").exists());
    // And the other direction: nothing may be moved *into* the outside dir.
    std::fs::write(a.share.join("payload"), b"x").unwrap();
    let mut body = fuse::as_bytes(&FuseRenameIn { newdir: parent }).to_vec();
    body.extend(two_names("payload", "dropped"));
    let err = request(&mut a.proc, FUSE_RENAME, 1, &body);
    assert!(err < 0, "rename into a symlink parent must fail");
    outside_intact(&a);
}

#[test]
fn mkdir_create_mknod_symlink_and_link_through_a_symlinked_parent_are_refused() {
    let mut a = attack("create");
    let parent = dirlink(&mut a);
    std::fs::write(a.share.join("inside.txt"), b"inside").unwrap();
    let inside = lookup(&mut a.proc, 1, "inside.txt").unwrap();

    let mut mkdir = fuse::as_bytes(&FuseMkdirIn { mode: 0o755, umask: 0 }).to_vec();
    mkdir.extend(name_body("newdir"));
    assert!(request(&mut a.proc, FUSE_MKDIR, parent, &mkdir) < 0);

    assert!(
        request(
            &mut a.proc,
            FUSE_CREATE,
            parent,
            &create_body(libc::O_WRONLY as u32, "dropped.txt")
        ) < 0
    );

    let mut mknod = fuse::as_bytes(&FuseMknodIn {
        mode: libc::S_IFREG | 0o644,
        rdev: 0,
        umask: 0,
        padding: 0,
    })
    .to_vec();
    mknod.extend(name_body("node"));
    assert!(request(&mut a.proc, FUSE_MKNOD, parent, &mknod) < 0);

    assert!(request(&mut a.proc, FUSE_SYMLINK, parent, &two_names("planted", "/etc/passwd")) < 0);

    let mut link = fuse::as_bytes(&FuseLinkIn { oldnodeid: inside }).to_vec();
    link.extend(name_body("linked"));
    assert!(request(&mut a.proc, FUSE_LINK, parent, &link) < 0);

    outside_intact(&a);
}

#[test]
fn opendir_and_lookup_through_a_symlinked_parent_do_not_list_outside() {
    let mut a = attack("opendir");
    let parent = dirlink(&mut a);
    assert!(
        open_dir(&mut a.proc, parent).is_err(),
        "opendir on a symlink inode must fail"
    );
    assert!(
        lookup(&mut a.proc, parent, "id_rsa").is_err(),
        "lookup below a symlink inode must fail"
    );
    // An in-share symlink is still a symlink: readlink works, traversal does not.
    std::fs::create_dir_all(a.share.join("real")).unwrap();
    std::fs::write(a.share.join("real/file"), b"ok").unwrap();
    std::os::unix::fs::symlink("real", a.share.join("alias")).unwrap();
    let alias = lookup(&mut a.proc, 1, "alias").unwrap();
    let resp = a
        .proc
        .handle_request(&build_request(&make_header(FUSE_READLINK, alias, 9), &[]));
    assert_eq!(response_error(&resp), 0);
    assert!(lookup(&mut a.proc, alias, "file").is_err());
    assert!(open_dir(&mut a.proc, alias).is_err());
}

#[test]
fn create_on_an_existing_symlink_does_not_truncate_the_target() {
    let mut a = attack("create-existing");
    std::os::unix::fs::symlink(a.outside.join("authorized_keys"), a.share.join("link")).unwrap();
    let err = request(
        &mut a.proc,
        FUSE_CREATE,
        1,
        &create_body((libc::O_WRONLY | libc::O_TRUNC) as u32, "link"),
    );
    assert!(err < 0, "create on a symlink to outside must fail");
    outside_intact(&a);
}

#[test]
fn setattr_on_a_symlink_inode_never_touches_the_target() {
    let mut a = attack("setattr");
    let target = a.outside.join("authorized_keys");
    let before = std::fs::metadata(&target).unwrap();
    std::os::unix::fs::symlink(&target, a.share.join("link")).unwrap();
    let ino = lookup(&mut a.proc, 1, "link").unwrap();

    assert!(request(&mut a.proc, FUSE_SETATTR, ino, &setattr_body(FATTR_SIZE, 0, 0, 0)) < 0);
    assert!(request(&mut a.proc, FUSE_SETATTR, ino, &setattr_body(FATTR_MODE, 0, 0o777, 0)) < 0);
    // Times on the link itself are legitimate (`touch -h`) and must stay on the link.
    let _ = request(
        &mut a.proc,
        FUSE_SETATTR,
        ino,
        &setattr_body(FATTR_MTIME, 0, 0, 1_000_000),
    );

    let after = std::fs::metadata(&target).unwrap();
    assert_eq!(after.len(), before.len());
    assert_eq!(after.mode() & 0o777, before.mode() & 0o777);
    assert_eq!(after.mtime(), before.mtime(), "target mtime must not change");
    outside_intact(&a);
}

#[test]
fn open_and_truncate_refuse_fifos_without_blocking() {
    let mut a = attack("fifo");
    let fifo = a.share.join("pipe");
    nix::unistd::mkfifo(&fifo, nix::sys::stat::Mode::from_bits_truncate(0o644)).unwrap();
    let ino = lookup(&mut a.proc, 1, "pipe").unwrap();

    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let open = open_file(&mut a.proc, ino, libc::O_RDONLY as u32);
        let truncate = request(&mut a.proc, FUSE_SETATTR, ino, &setattr_body(FATTR_SIZE, 0, 0, 0));
        let _ = tx.send((open, truncate));
    });
    let (open, truncate) = rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("a FIFO must not park the FUSE worker");
    assert!(open.is_err(), "open on a FIFO must be refused");
    assert!(truncate < 0, "truncate on a FIFO must be refused");
}

#[test]
fn a_parent_replaced_by_a_symlink_after_lookup_is_refused() {
    let mut a = attack("swap");
    std::fs::create_dir_all(a.share.join("sub")).unwrap();
    let sub = lookup(&mut a.proc, 1, "sub").unwrap();
    // The guest (or anything) swaps the directory for a link after lookup.
    std::fs::remove_dir(a.share.join("sub")).unwrap();
    std::os::unix::fs::symlink(&a.outside, a.share.join("sub")).unwrap();

    assert!(request(&mut a.proc, FUSE_UNLINK, sub, &name_body("authorized_keys")) < 0);
    assert!(
        request(
            &mut a.proc,
            FUSE_CREATE,
            sub,
            &create_body(libc::O_WRONLY as u32, "dropped")
        ) < 0
    );
    assert!(open_dir(&mut a.proc, sub).is_err());
    outside_intact(&a);
}

#[test]
fn legitimate_namespace_operations_still_work() {
    let mut a = attack("legit");
    let mut mkdir = fuse::as_bytes(&FuseMkdirIn { mode: 0o755, umask: 0 }).to_vec();
    mkdir.extend(name_body("d"));
    assert_eq!(request(&mut a.proc, FUSE_MKDIR, 1, &mkdir), 0);
    let d = lookup(&mut a.proc, 1, "d").unwrap();
    assert_eq!(
        request(&mut a.proc, FUSE_CREATE, d, &create_body(libc::O_WRONLY as u32, "f")),
        0
    );
    let f = lookup(&mut a.proc, d, "f").unwrap();
    assert!(open_file(&mut a.proc, f, libc::O_RDONLY as u32).is_ok());
    let mut body = fuse::as_bytes(&FuseRenameIn { newdir: 1 }).to_vec();
    body.extend(two_names("f", "g"));
    assert_eq!(request(&mut a.proc, FUSE_RENAME, d, &body), 0);
    assert!(a.share.join("g").exists());
    assert_eq!(request(&mut a.proc, FUSE_SYMLINK, 1, &two_names("l", "g")), 0);
    assert_eq!(request(&mut a.proc, FUSE_UNLINK, 1, &name_body("g")), 0);
    assert!(open_dir(&mut a.proc, d).is_ok());
    let _ = Path::new("");
}
