//! Opcode numbering and ownership, as the guest kernel and guest tools see them.
//!
//! FUSE_FSYNCDIR was numbered 21, which the kernel uses for SETXATTR: every
//! guest setxattr was parsed as an fsyncdir and failed with EBADF, and `cp -a`
//! into the workspace failed "preserving permissions". A guest chown reached an
//! unprivileged host lchown and failed with EPERM. The share reports every
//! entry as 0:0 and must never change host ownership, so chown is accepted
//! without effect.

use std::os::unix::fs::MetadataExt;

use super::tests::{build_request, lookup, make_header, open_dir, response_error, temp_share, test_processor};
use crate::hypervisor::fuse::{self, *};

/// The numbering of include/uapi/linux/fuse.h, which the guest kernel speaks.
const LINUX_OPCODES: &[(u32, u32)] = &[
    (FUSE_LOOKUP, 1),
    (FUSE_FORGET, 2),
    (FUSE_GETATTR, 3),
    (FUSE_SETATTR, 4),
    (FUSE_READLINK, 5),
    (FUSE_SYMLINK, 6),
    (FUSE_MKNOD, 8),
    (FUSE_MKDIR, 9),
    (FUSE_UNLINK, 10),
    (FUSE_RMDIR, 11),
    (FUSE_RENAME, 12),
    (FUSE_LINK, 13),
    (FUSE_OPEN, 14),
    (FUSE_READ, 15),
    (FUSE_WRITE, 16),
    (FUSE_STATFS, 17),
    (FUSE_RELEASE, 18),
    (FUSE_FSYNC, 20),
    (FUSE_FLUSH, 25),
    (FUSE_INIT, 26),
    (FUSE_OPENDIR, 27),
    (FUSE_READDIR, 28),
    (FUSE_RELEASEDIR, 29),
    (FUSE_FSYNCDIR, 30),
    (FUSE_CREATE, 35),
    (FUSE_BATCH_FORGET, 42),
    (FUSE_RENAME2, 45),
    (FUSE_LSEEK, 46),
];
const LINUX_SETXATTR: u32 = 21;
const LINUX_GETXATTR: u32 = 22;
const LINUX_LISTXATTR: u32 = 23;
const LINUX_REMOVEXATTR: u32 = 24;

#[test]
fn every_opcode_matches_the_guest_kernel_numbering() {
    for &(ours, linux) in LINUX_OPCODES {
        assert_eq!(ours, linux, "opcode {ours} is {linux} in the guest kernel");
    }
}

#[test]
fn xattr_requests_are_unsupported_not_misrouted() {
    let dir = temp_share("xattr-enosys");
    std::fs::write(dir.join("f"), b"x").unwrap();
    let mut proc = test_processor(&dir);
    let ino = lookup(&mut proc, 1, "f").unwrap();
    // ENOSYS makes the guest kernel report EOPNOTSUPP, which `cp -a`, tar and
    // rsync treat as "this filesystem has no xattrs" rather than a failure.
    for opcode in [LINUX_SETXATTR, LINUX_GETXATTR, LINUX_LISTXATTR, LINUX_REMOVEXATTR] {
        let body = b"\x01\x00\x00\x00\x00\x00\x00\x00user.x\0value";
        let resp = proc.handle_request(&build_request(&make_header(opcode, ino, 7), body));
        assert_eq!(response_error(&resp), -libc::ENOSYS, "opcode {opcode}");
    }
}

#[test]
fn fsyncdir_is_served_on_its_own_opcode() {
    let dir = temp_share("fsyncdir-30");
    let mut proc = test_processor(&dir);
    let fh = open_dir(&mut proc, 1).unwrap();
    let body = FuseFsyncIn {
        fh,
        fsync_flags: 0,
        padding: 0,
    };
    let resp = proc.handle_request(&build_request(&make_header(30, 1, 8), fuse::as_bytes(&body)));
    assert_eq!(response_error(&resp), 0);
}

fn chown_request(ino: u64, valid: u32, uid: u32, gid: u32) -> Vec<u8> {
    let attr_in = FuseSetAttrIn {
        valid,
        padding: 0,
        fh: 0,
        size: 0,
        lock_owner: 0,
        atime: 0,
        mtime: 0,
        ctime: 0,
        atimensec: 0,
        mtimensec: 0,
        ctimensec: 0,
        mode: 0,
        unused4: 0,
        uid,
        gid,
        unused5: 0,
    };
    build_request(&make_header(FUSE_SETATTR, ino, 9), fuse::as_bytes(&attr_in))
}

#[test]
fn chown_is_accepted_and_never_changes_host_ownership() {
    let dir = temp_share("chown-noop");
    std::fs::write(dir.join("f"), b"x").unwrap();
    let before = std::fs::metadata(dir.join("f")).unwrap();
    let mut proc = test_processor(&dir);
    let ino = lookup(&mut proc, 1, "f").unwrap();

    // Root, a user, the "unchanged" sentinel and the extremes a hostile guest
    // might send: none may reach the host or be refused.
    for (valid, uid, gid) in [
        (FATTR_UID | FATTR_GID, 0, 0),
        (FATTR_UID | FATTR_GID, 1000, 1000),
        (FATTR_UID, 65534, 0),
        (FATTR_GID, 0, u32::MAX),
        (FATTR_UID | FATTR_GID, u32::MAX - 1, u32::MAX - 1),
    ] {
        let resp = proc.handle_request(&chown_request(ino, valid, uid, gid));
        assert_eq!(response_error(&resp), 0, "chown {uid}:{gid}");
        let after = std::fs::metadata(dir.join("f")).unwrap();
        assert_eq!((after.uid(), after.gid()), (before.uid(), before.gid()));
    }
}

#[test]
fn chown_alongside_a_mode_change_still_applies_the_mode() {
    use std::os::unix::fs::PermissionsExt;
    let dir = temp_share("chown-mode");
    std::fs::write(dir.join("f"), b"x").unwrap();
    let mut proc = test_processor(&dir);
    let ino = lookup(&mut proc, 1, "f").unwrap();
    let attr_in = FuseSetAttrIn {
        valid: FATTR_UID | FATTR_GID | FATTR_MODE,
        padding: 0,
        fh: 0,
        size: 0,
        lock_owner: 0,
        atime: 0,
        mtime: 0,
        ctime: 0,
        atimensec: 0,
        mtimensec: 0,
        ctimensec: 0,
        mode: 0o640,
        unused4: 0,
        uid: 1000,
        gid: 1000,
        unused5: 0,
    };
    let request = build_request(&make_header(FUSE_SETATTR, ino, 10), fuse::as_bytes(&attr_in));
    let resp = proc.handle_request(&request);
    assert_eq!(response_error(&resp), 0);
    let mode = std::fs::metadata(dir.join("f")).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o640);
}

/// Ownership is derived from each request's caller, so the kernel must ask
/// again when a different namespace reaches an inode that VM root touched.
#[test]
fn caller_owned_attributes_are_never_cached() {
    let dir = temp_share("caller-owned-attributes");
    std::fs::write(dir.join("f"), b"x").unwrap();
    let mut proc = test_processor(&dir);

    let mut lookup_header = make_header(FUSE_LOOKUP, 1, 11);
    lookup_header.uid = 1000;
    lookup_header.gid = 1000;
    let lookup_response = proc.handle_request(&build_request(&lookup_header, b"f\0"));
    let payload = std::mem::size_of::<FuseOutHeader>();
    let entry: FuseEntryOut = fuse::read_struct(&lookup_response[payload..]).unwrap();
    assert_eq!((entry.attr.uid, entry.attr.gid), (1000, 1000));
    assert_eq!(entry.attr_valid, 0);
    assert_eq!(entry.attr_valid_nsec, 0);

    let mut getattr_header = make_header(FUSE_GETATTR, entry.nodeid, 12);
    getattr_header.uid = 2000;
    getattr_header.gid = 2000;
    let getattr_response = proc.handle_request(&build_request(&getattr_header, &[]));
    let attr: FuseAttrOut = fuse::read_struct(&getattr_response[payload..]).unwrap();
    assert_eq!((attr.attr.uid, attr.attr.gid), (2000, 2000));
    assert_eq!(attr.attr_valid, 0);
    assert_eq!(attr.attr_valid_nsec, 0);
}
