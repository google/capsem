//! A read-only share is read-only in the device, not in the guest's mount.
//!
//! The image share (`session::IMAGE_SHARE_TAG`) is attached read-only so a
//! guest root that mounts it without `ro`, or remounts it read-write, still
//! cannot change a byte: the guest kernel's mount flags never reach this
//! server, so every mutating request must be refused here, whatever the
//! guest sends. One request of every mutating opcode, against a device built
//! the way `KvmHypervisor::boot` builds it from a `read_only` share.

use std::sync::atomic::AtomicU32;
use std::sync::Arc;

use super::tests::{build_request, lookup, make_header, open_file, response_error, temp_share};
use super::VirtioFsDevice;
use crate::hypervisor::fuse::{self, *};

fn name(body: &mut Vec<u8>, names: &[&str]) {
    for name in names {
        body.extend_from_slice(name.as_bytes());
        body.push(0);
    }
}

#[test]
fn a_read_only_share_refuses_every_mutating_request() {
    let dir = temp_share("image-share-read-only");
    std::fs::write(dir.join("blob"), b"verified").unwrap();
    std::fs::create_dir(dir.join("directory")).unwrap();
    let mut device = VirtioFsDevice::new("capsem-image", &dir, true, None, -1, Arc::new(AtomicU32::new(0))).unwrap();
    let mut proc = device
        .processor
        .take()
        .expect("an unactivated device owns its processor");
    assert!(proc.read_only, "the share's read_only reaches the server");
    let blob = lookup(&mut proc, 1, "blob").unwrap();

    // Reading still works: the launcher unpacks from this share.
    let fh = open_file(&mut proc, blob, libc::O_RDONLY as u32).unwrap();
    assert!(fh != 0);
    for flags in [libc::O_WRONLY, libc::O_RDWR, libc::O_WRONLY | libc::O_TRUNC] {
        assert_eq!(
            open_file(&mut proc, blob, flags as u32).unwrap_err(),
            -libc::EROFS,
            "{flags:#x}"
        );
    }
    // A read-only open asking to truncate is refused too (the host open
    // refuses truncation without write access), and truncates nothing.
    assert!(open_file(&mut proc, blob, (libc::O_RDONLY | libc::O_TRUNC) as u32).is_err());
    assert_eq!(std::fs::read(dir.join("blob")).unwrap(), b"verified");

    let mut requests: Vec<(&str, u32, u64, Vec<u8>)> = Vec::new();
    let setattr = FuseSetAttrIn {
        valid: FATTR_MODE | FATTR_SIZE,
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
        mode: 0o777,
        unused4: 0,
        uid: 0,
        gid: 0,
        unused5: 0,
    };
    requests.push(("setattr", FUSE_SETATTR, blob, fuse::as_bytes(&setattr).to_vec()));
    let write = FuseWriteIn {
        fh,
        offset: 0,
        size: 3,
        write_flags: 0,
        lock_owner: 0,
        flags: 0,
        padding: 0,
    };
    let mut body = fuse::as_bytes(&write).to_vec();
    body.extend_from_slice(b"bad");
    requests.push(("write", FUSE_WRITE, blob, body));
    let create = FuseCreateIn {
        flags: libc::O_RDWR as u32,
        mode: 0o644,
        umask: 0,
        open_flags: 0,
    };
    let mut body = fuse::as_bytes(&create).to_vec();
    name(&mut body, &["planted"]);
    requests.push(("create", FUSE_CREATE, 1, body));
    let mknod = FuseMknodIn {
        mode: libc::S_IFIFO | 0o644,
        rdev: 0,
        umask: 0,
        padding: 0,
    };
    let mut body = fuse::as_bytes(&mknod).to_vec();
    name(&mut body, &["fifo"]);
    requests.push(("mknod", FUSE_MKNOD, 1, body));
    let mkdir = FuseMkdirIn { mode: 0o755, umask: 0 };
    let mut body = fuse::as_bytes(&mkdir).to_vec();
    name(&mut body, &["made"]);
    requests.push(("mkdir", FUSE_MKDIR, 1, body));
    let mut body = Vec::new();
    name(&mut body, &["blob"]);
    requests.push(("unlink", FUSE_UNLINK, 1, body));
    let mut body = Vec::new();
    name(&mut body, &["directory"]);
    requests.push(("rmdir", FUSE_RMDIR, 1, body));
    let mut body = fuse::as_bytes(&FuseRenameIn { newdir: 1 }).to_vec();
    name(&mut body, &["blob", "moved"]);
    requests.push(("rename", FUSE_RENAME, 1, body));
    let rename2 = FuseRename2In {
        newdir: 1,
        flags: 0,
        padding: 0,
    };
    let mut body = fuse::as_bytes(&rename2).to_vec();
    name(&mut body, &["blob", "moved2"]);
    requests.push(("rename2", FUSE_RENAME2, 1, body));
    let mut body = Vec::new();
    name(&mut body, &["symlink", "/etc/shadow"]);
    requests.push(("symlink", FUSE_SYMLINK, 1, body));
    let mut body = fuse::as_bytes(&FuseLinkIn { oldnodeid: blob }).to_vec();
    name(&mut body, &["hardlink"]);
    requests.push(("link", FUSE_LINK, 1, body));

    for (unique, (label, opcode, node, body)) in requests.into_iter().enumerate() {
        let header = make_header(opcode, node, 100 + unique as u64);
        let response = proc.handle_request(&build_request(&header, &body));
        assert_eq!(response_error(&response), -libc::EROFS, "{label} was not refused");
    }

    let mut left: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    left.sort();
    assert_eq!(left, ["blob", "directory"], "the share changed");
    assert_eq!(std::fs::read(dir.join("blob")).unwrap(), b"verified");
}
