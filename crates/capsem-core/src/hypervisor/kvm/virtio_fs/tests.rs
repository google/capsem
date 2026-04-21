use super::*;

fn temp_share(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("capsem-virtfs-test").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Helper: create a FuseProcessor for testing (no queues needed).
fn test_processor(dir: &Path) -> FuseProcessor {
    FuseProcessor {
        root_path: dir.to_path_buf(),
        read_only: false,
        inodes: InodeTable::new(dir).unwrap(),
        file_handles: FileHandleTable::new(),
    }
}

#[test]
fn fs_device_type() {
    let dir = temp_share("dev-type");
    assert_eq!(VirtioFsDevice::new("capsem", &dir, false, -1).unwrap().device_type(), VIRTIO_ID_FS);
}

#[test]
fn fs_features() {
    let dir = temp_share("features");
    assert_ne!(VirtioFsDevice::new("capsem", &dir, false, -1).unwrap().features() & VIRTIO_F_VERSION_1, 0);
}

#[test]
fn fs_two_queues() {
    let dir = temp_share("queues");
    assert_eq!(VirtioFsDevice::new("capsem", &dir, false, -1).unwrap().queue_max_sizes(), &[QUEUE_SIZE, QUEUE_SIZE]);
}

#[test]
fn fs_config_tag() {
    let dir = temp_share("cfg-tag");
    let dev = VirtioFsDevice::new("capsem", &dir, false, -1).unwrap();
    let mut data = [0u8; 36];
    dev.read_config(0, &mut data);
    assert_eq!(&data[..6], b"capsem");
    assert!(data[6..].iter().all(|&b| b == 0));
}

#[test]
fn fs_config_nrq() {
    let dir = temp_share("cfg-nrq");
    let dev = VirtioFsDevice::new("capsem", &dir, false, -1).unwrap();
    let mut data = [0u8; 4];
    dev.read_config(36, &mut data);
    assert_eq!(u32::from_le_bytes(data), 1);
}

#[test]
fn fs_config_past_end() {
    let dir = temp_share("cfg-past");
    let dev = VirtioFsDevice::new("capsem", &dir, false, -1).unwrap();
    let mut data = [0xFFu8; 4];
    dev.read_config(40, &mut data);
    assert!(data.iter().all(|&b| b == 0));
}

#[test]
fn init_response_version() {
    let dir = temp_share("init-ver");
    let mut proc = test_processor(&dir);
    let header = FuseInHeader {
        len: 56, opcode: FUSE_INIT, unique: 1,
        nodeid: 0, uid: 0, gid: 0, pid: 0, padding: 0,
    };
    let init_in = FuseInitIn { major: 7, minor: 38, max_readahead: 131072, flags: 0 };
    let mut req = fuse::as_bytes(&header).to_vec();
    req.extend_from_slice(fuse::as_bytes(&init_in));

    let resp = proc.handle_request(&req);
    let out: FuseOutHeader = fuse::read_struct(&resp).unwrap();
    assert_eq!(out.error, 0);
    let init_out: FuseInitOut = fuse::read_struct(&resp[16..]).unwrap();
    assert_eq!(init_out.major, 7);
    assert_eq!(init_out.minor, 31);
    assert!(init_out.max_write > 0);
}

// ── Test helpers ─────────────────────────────────────────────────

const HDR_SIZE: usize = std::mem::size_of::<FuseInHeader>();
const OUT_HDR_SIZE: usize = std::mem::size_of::<FuseOutHeader>();
const ENTRY_OUT_SIZE: usize = std::mem::size_of::<FuseEntryOut>();
const ATTR_OUT_SIZE: usize = std::mem::size_of::<FuseAttrOut>();

fn make_header(opcode: u32, nodeid: u64, unique: u64) -> FuseInHeader {
    FuseInHeader { len: 0, opcode, unique, nodeid, uid: 0, gid: 0, pid: 0, padding: 0 }
}

fn build_request(header: &FuseInHeader, body: &[u8]) -> Vec<u8> {
    let mut req = fuse::as_bytes(header).to_vec();
    req.extend_from_slice(body);
    req
}

fn response_error(resp: &[u8]) -> i32 {
    fuse::read_struct::<FuseOutHeader>(resp).unwrap().error
}

/// LOOKUP a name under a parent inode, return the entry's nodeid.
fn lookup(proc: &mut FuseProcessor, parent: u64, name: &str) -> Result<u64, i32> {
    let h = make_header(FUSE_LOOKUP, parent, 100);
    let mut body = name.as_bytes().to_vec();
    body.push(0);
    let resp = proc.handle_request(&build_request(&h, &body));
    let out: FuseOutHeader = fuse::read_struct(&resp).unwrap();
    if out.error != 0 { return Err(out.error); }
    let entry: FuseEntryOut = fuse::read_struct(&resp[OUT_HDR_SIZE..]).unwrap();
    Ok(entry.nodeid)
}

/// OPEN a file by inode, return the file handle.
fn open_file(proc: &mut FuseProcessor, nodeid: u64, flags: u32) -> Result<u64, i32> {
    let h = make_header(FUSE_OPEN, nodeid, 200);
    let open_in = FuseOpenIn { flags, open_flags: 0 };
    let resp = proc.handle_request(&build_request(&h, fuse::as_bytes(&open_in)));
    let out: FuseOutHeader = fuse::read_struct(&resp).unwrap();
    if out.error != 0 { return Err(out.error); }
    let open_out: FuseOpenOut = fuse::read_struct(&resp[OUT_HDR_SIZE..]).unwrap();
    Ok(open_out.fh)
}

// ── ops_meta tests ───────────────────────────────────────────────

#[test]
fn lookup_existing_file() {
    let dir = temp_share("lookup-exist");
    std::fs::write(dir.join("hello.txt"), b"data").unwrap();
    let mut proc = test_processor(&dir);
    let ino = lookup(&mut proc, 1, "hello.txt").unwrap();
    assert!(ino > 1, "lookup should return a valid inode");
}

#[test]
fn lookup_nonexistent() {
    let dir = temp_share("lookup-none");
    let mut proc = test_processor(&dir);
    let err = lookup(&mut proc, 1, "nope.txt").unwrap_err();
    assert_eq!(err, -libc::ENOENT);
}

#[test]
fn getattr_root() {
    let dir = temp_share("getattr-root");
    let mut proc = test_processor(&dir);
    let h = make_header(FUSE_GETATTR, 1, 1);
    let resp = proc.handle_request(&build_request(&h, &[]));
    assert_eq!(response_error(&resp), 0);
    let attr_out: FuseAttrOut = fuse::read_struct(&resp[OUT_HDR_SIZE..]).unwrap();
    assert_ne!(attr_out.attr.mode & S_IFDIR, 0, "root should be a directory");
}

#[test]
fn getattr_file() {
    let dir = temp_share("getattr-file");
    std::fs::write(dir.join("f.txt"), b"12345").unwrap();
    let mut proc = test_processor(&dir);
    let ino = lookup(&mut proc, 1, "f.txt").unwrap();

    let h = make_header(FUSE_GETATTR, ino, 2);
    let resp = proc.handle_request(&build_request(&h, &[]));
    assert_eq!(response_error(&resp), 0);
    let attr_out: FuseAttrOut = fuse::read_struct(&resp[OUT_HDR_SIZE..]).unwrap();
    assert_eq!(attr_out.attr.size, 5);
    assert_ne!(attr_out.attr.mode & S_IFREG, 0);
}

#[test]
fn getattr_nonexistent_inode() {
    let dir = temp_share("getattr-bad");
    let mut proc = test_processor(&dir);
    let h = make_header(FUSE_GETATTR, 99999, 1);
    let resp = proc.handle_request(&build_request(&h, &[]));
    assert_eq!(response_error(&resp), -libc::ENOENT);
}

#[test]
fn setattr_chmod() {
    let dir = temp_share("setattr-chmod");
    std::fs::write(dir.join("f.txt"), b"x").unwrap();
    let mut proc = test_processor(&dir);
    let ino = lookup(&mut proc, 1, "f.txt").unwrap();

    let attr_in = FuseSetAttrIn {
        valid: FATTR_MODE, padding: 0, fh: 0, size: 0,
        lock_owner: 0, atime: 0, mtime: 0, ctime: 0,
        atimensec: 0, mtimensec: 0, ctimensec: 0,
        mode: 0o755, unused4: 0, uid: 0, gid: 0, unused5: 0,
    };
    let h = make_header(FUSE_SETATTR, ino, 3);
    let resp = proc.handle_request(&build_request(&h, fuse::as_bytes(&attr_in)));
    assert_eq!(response_error(&resp), 0);

    let perms = std::fs::metadata(dir.join("f.txt")).unwrap().permissions();
    assert_eq!(perms.mode() & 0o777, 0o755);
}

#[test]
fn setattr_truncate() {
    let dir = temp_share("setattr-trunc");
    std::fs::write(dir.join("big.txt"), b"hello world").unwrap();
    let mut proc = test_processor(&dir);
    let ino = lookup(&mut proc, 1, "big.txt").unwrap();

    let attr_in = FuseSetAttrIn {
        valid: FATTR_SIZE, padding: 0, fh: 0, size: 3,
        lock_owner: 0, atime: 0, mtime: 0, ctime: 0,
        atimensec: 0, mtimensec: 0, ctimensec: 0,
        mode: 0, unused4: 0, uid: 0, gid: 0, unused5: 0,
    };
    let h = make_header(FUSE_SETATTR, ino, 4);
    let resp = proc.handle_request(&build_request(&h, fuse::as_bytes(&attr_in)));
    assert_eq!(response_error(&resp), 0);

    let content = std::fs::read(dir.join("big.txt")).unwrap();
    assert_eq!(content.len(), 3);
    assert_eq!(&content, b"hel");
}

#[test]
fn setattr_read_only_rejected() {
    let dir = temp_share("setattr-ro");
    std::fs::write(dir.join("f.txt"), b"x").unwrap();
    let mut proc = test_processor(&dir);
    proc.read_only = true;
    let ino = lookup(&mut proc, 1, "f.txt").unwrap();

    let attr_in = FuseSetAttrIn {
        valid: FATTR_MODE, padding: 0, fh: 0, size: 0,
        lock_owner: 0, atime: 0, mtime: 0, ctime: 0,
        atimensec: 0, mtimensec: 0, ctimensec: 0,
        mode: 0o777, unused4: 0, uid: 0, gid: 0, unused5: 0,
    };
    let h = make_header(FUSE_SETATTR, ino, 5);
    let resp = proc.handle_request(&build_request(&h, fuse::as_bytes(&attr_in)));
    assert_eq!(response_error(&resp), -libc::EROFS);
}

#[test]
fn statfs_returns_data() {
    let dir = temp_share("statfs");
    let mut proc = test_processor(&dir);
    let h = make_header(FUSE_STATFS, 1, 1);
    let resp = proc.handle_request(&build_request(&h, &[]));
    assert_eq!(response_error(&resp), 0);
    let kstatfs: FuseKStatfs = fuse::read_struct(&resp[OUT_HDR_SIZE..]).unwrap();
    assert!(kstatfs.blocks > 0, "statfs should report non-zero blocks");
    assert!(kstatfs.bsize > 0, "statfs should report non-zero block size");
}

#[test]
fn forget_does_not_crash() {
    let dir = temp_share("forget");
    std::fs::write(dir.join("f.txt"), b"x").unwrap();
    let mut proc = test_processor(&dir);
    let ino = lookup(&mut proc, 1, "f.txt").unwrap();

    // FORGET for a valid inode
    let h = make_header(FUSE_FORGET, ino, 1);
    let forget_in = FuseForgetIn { nlookup: 1 };
    let resp = proc.handle_request(&build_request(&h, fuse::as_bytes(&forget_in)));
    assert!(resp.is_empty(), "FORGET should produce no response");

    // FORGET for a nonexistent inode -- should not panic
    let h2 = make_header(FUSE_FORGET, 99999, 2);
    let resp2 = proc.handle_request(&build_request(&h2, fuse::as_bytes(&forget_in)));
    assert!(resp2.is_empty());
}

#[test]
fn batch_forget_multiple() {
    let dir = temp_share("batch-forget");
    std::fs::write(dir.join("a.txt"), b"a").unwrap();
    std::fs::write(dir.join("b.txt"), b"b").unwrap();
    let mut proc = test_processor(&dir);
    let ino_a = lookup(&mut proc, 1, "a.txt").unwrap();
    let ino_b = lookup(&mut proc, 1, "b.txt").unwrap();

    let h = make_header(FUSE_BATCH_FORGET, 0, 1);
    let batch = FuseBatchForgetIn { count: 2, dummy: 0 };
    let e1 = FuseForgetOne { nodeid: ino_a, nlookup: 1 };
    let e2 = FuseForgetOne { nodeid: ino_b, nlookup: 1 };
    let mut body = fuse::as_bytes(&batch).to_vec();
    body.extend_from_slice(fuse::as_bytes(&e1));
    body.extend_from_slice(fuse::as_bytes(&e2));

    let resp = proc.handle_request(&build_request(&h, &body));
    assert!(resp.is_empty(), "BATCH_FORGET should produce no response");
}

// ── ops_file tests ───────────────────────────────────────────────

#[test]
fn open_read_write_release() {
    let dir = temp_share("file-lifecycle");
    std::fs::write(dir.join("rw.txt"), b"initial").unwrap();
    let mut proc = test_processor(&dir);
    let ino = lookup(&mut proc, 1, "rw.txt").unwrap();

    // OPEN for read+write
    let fh = open_file(&mut proc, ino, libc::O_RDWR as u32).unwrap();

    // READ
    let read_in = FuseReadIn {
        fh, offset: 0, size: 1024, read_flags: 0,
        lock_owner: 0, flags: 0, padding: 0,
    };
    let h = make_header(FUSE_READ, ino, 10);
    let resp = proc.handle_request(&build_request(&h, fuse::as_bytes(&read_in)));
    assert_eq!(response_error(&resp), 0);
    assert_eq!(&resp[OUT_HDR_SIZE..], b"initial");

    // WRITE at offset 0
    let write_in = FuseWriteIn {
        fh, offset: 0, size: 7, write_flags: 0,
        lock_owner: 0, flags: 0, padding: 0,
    };
    let h = make_header(FUSE_WRITE, ino, 11);
    let mut body = fuse::as_bytes(&write_in).to_vec();
    body.extend_from_slice(b"updated");
    let resp = proc.handle_request(&build_request(&h, &body));
    assert_eq!(response_error(&resp), 0);
    let write_out: FuseWriteOut = fuse::read_struct(&resp[OUT_HDR_SIZE..]).unwrap();
    assert_eq!(write_out.size, 7);

    // RELEASE
    let release_in = FuseReleaseIn { fh, flags: 0, release_flags: 0, lock_owner: 0 };
    let h = make_header(FUSE_RELEASE, ino, 12);
    let resp = proc.handle_request(&build_request(&h, fuse::as_bytes(&release_in)));
    assert_eq!(response_error(&resp), 0);

    // Verify on disk
    assert_eq!(std::fs::read(dir.join("rw.txt")).unwrap(), b"updated");
}

#[test]
fn open_nonexistent() {
    let dir = temp_share("open-none");
    let mut proc = test_processor(&dir);
    let err = open_file(&mut proc, 99999, libc::O_RDONLY as u32).unwrap_err();
    assert_eq!(err, -libc::ENOENT);
}

#[test]
fn open_write_on_readonly() {
    let dir = temp_share("open-ro");
    std::fs::write(dir.join("f.txt"), b"x").unwrap();
    let mut proc = test_processor(&dir);
    proc.read_only = true;
    let ino = lookup(&mut proc, 1, "f.txt").unwrap();
    let err = open_file(&mut proc, ino, libc::O_WRONLY as u32).unwrap_err();
    assert_eq!(err, -libc::EROFS);
}

#[test]
fn read_with_offset() {
    let dir = temp_share("read-offset");
    std::fs::write(dir.join("data.txt"), b"abcdefghij").unwrap();
    let mut proc = test_processor(&dir);
    let ino = lookup(&mut proc, 1, "data.txt").unwrap();
    let fh = open_file(&mut proc, ino, libc::O_RDONLY as u32).unwrap();

    let read_in = FuseReadIn {
        fh, offset: 5, size: 100, read_flags: 0,
        lock_owner: 0, flags: 0, padding: 0,
    };
    let h = make_header(FUSE_READ, ino, 1);
    let resp = proc.handle_request(&build_request(&h, fuse::as_bytes(&read_in)));
    assert_eq!(response_error(&resp), 0);
    assert_eq!(&resp[OUT_HDR_SIZE..], b"fghij");
}

#[test]
fn read_past_eof_returns_empty() {
    let dir = temp_share("read-eof");
    std::fs::write(dir.join("small.txt"), b"hi").unwrap();
    let mut proc = test_processor(&dir);
    let ino = lookup(&mut proc, 1, "small.txt").unwrap();
    let fh = open_file(&mut proc, ino, libc::O_RDONLY as u32).unwrap();

    let read_in = FuseReadIn {
        fh, offset: 100, size: 100, read_flags: 0,
        lock_owner: 0, flags: 0, padding: 0,
    };
    let h = make_header(FUSE_READ, ino, 1);
    let resp = proc.handle_request(&build_request(&h, fuse::as_bytes(&read_in)));
    assert_eq!(response_error(&resp), 0);
    assert_eq!(resp.len(), OUT_HDR_SIZE, "read past EOF should return empty body");
}

#[test]
fn write_on_readonly_rejected() {
    let dir = temp_share("write-ro");
    std::fs::write(dir.join("f.txt"), b"x").unwrap();
    let mut proc = test_processor(&dir);
    let ino = lookup(&mut proc, 1, "f.txt").unwrap();
    proc.read_only = true;

    let write_in = FuseWriteIn {
        fh: 0, offset: 0, size: 3, write_flags: 0,
        lock_owner: 0, flags: 0, padding: 0,
    };
    let h = make_header(FUSE_WRITE, ino, 1);
    let mut body = fuse::as_bytes(&write_in).to_vec();
    body.extend_from_slice(b"bad");
    let resp = proc.handle_request(&build_request(&h, &body));
    assert_eq!(response_error(&resp), -libc::EROFS);
}

#[test]
fn create_new_file() {
    let dir = temp_share("create-new");
    let mut proc = test_processor(&dir);

    let create_in = FuseCreateIn {
        flags: libc::O_RDWR as u32, mode: 0o644, umask: 0, open_flags: 0,
    };
    let h = make_header(FUSE_CREATE, 1, 1);
    let mut body = fuse::as_bytes(&create_in).to_vec();
    body.extend_from_slice(b"newfile.txt\0");
    let resp = proc.handle_request(&build_request(&h, &body));
    assert_eq!(response_error(&resp), 0);

    // File should exist on disk
    assert!(dir.join("newfile.txt").exists());

    // Response should contain entry + open
    let entry: FuseEntryOut = fuse::read_struct(&resp[OUT_HDR_SIZE..]).unwrap();
    assert!(entry.nodeid > 0);
    let open_out: FuseOpenOut = fuse::read_struct(
        &resp[OUT_HDR_SIZE + ENTRY_OUT_SIZE..]
    ).unwrap();
    assert!(open_out.fh > 0);
}

#[test]
fn create_readonly_rejected() {
    let dir = temp_share("create-ro");
    let mut proc = test_processor(&dir);
    proc.read_only = true;

    let create_in = FuseCreateIn {
        flags: libc::O_RDWR as u32, mode: 0o644, umask: 0, open_flags: 0,
    };
    let h = make_header(FUSE_CREATE, 1, 1);
    let mut body = fuse::as_bytes(&create_in).to_vec();
    body.extend_from_slice(b"nope.txt\0");
    let resp = proc.handle_request(&build_request(&h, &body));
    assert_eq!(response_error(&resp), -libc::EROFS);
    assert!(!dir.join("nope.txt").exists());
}

#[test]
fn create_existing_file_opens_it() {
    let dir = temp_share("create-exist");
    std::fs::write(dir.join("exist.txt"), b"old content").unwrap();
    let mut proc = test_processor(&dir);

    let create_in = FuseCreateIn {
        flags: libc::O_RDWR as u32, mode: 0o644, umask: 0, open_flags: 0,
    };
    let h = make_header(FUSE_CREATE, 1, 1);
    let mut body = fuse::as_bytes(&create_in).to_vec();
    body.extend_from_slice(b"exist.txt\0");
    let resp = proc.handle_request(&build_request(&h, &body));
    assert_eq!(response_error(&resp), 0);
}

#[test]
fn flush_and_fsync() {
    let dir = temp_share("flush-fsync");
    std::fs::write(dir.join("f.txt"), b"data").unwrap();
    let mut proc = test_processor(&dir);
    let ino = lookup(&mut proc, 1, "f.txt").unwrap();
    let fh = open_file(&mut proc, ino, libc::O_RDWR as u32).unwrap();

    // FLUSH
    let flush_in = FuseFlushIn { fh, unused: 0, padding: 0, lock_owner: 0 };
    let h = make_header(FUSE_FLUSH, ino, 1);
    let resp = proc.handle_request(&build_request(&h, fuse::as_bytes(&flush_in)));
    assert_eq!(response_error(&resp), 0);

    // FSYNC (data-only)
    let fsync_in = FuseFsyncIn { fh, fsync_flags: 1, padding: 0 };
    let h = make_header(FUSE_FSYNC, ino, 2);
    let resp = proc.handle_request(&build_request(&h, fuse::as_bytes(&fsync_in)));
    assert_eq!(response_error(&resp), 0);

    // FSYNC (full)
    let fsync_in = FuseFsyncIn { fh, fsync_flags: 0, padding: 0 };
    let h = make_header(FUSE_FSYNC, ino, 3);
    let resp = proc.handle_request(&build_request(&h, fuse::as_bytes(&fsync_in)));
    assert_eq!(response_error(&resp), 0);
}

#[test]
fn flush_bad_handle() {
    let dir = temp_share("flush-bad");
    let mut proc = test_processor(&dir);
    let flush_in = FuseFlushIn { fh: 99999, unused: 0, padding: 0, lock_owner: 0 };
    let h = make_header(FUSE_FLUSH, 1, 1);
    let resp = proc.handle_request(&build_request(&h, fuse::as_bytes(&flush_in)));
    assert_eq!(response_error(&resp), -libc::EBADF);
}

#[test]
fn lseek_whence() {
    let dir = temp_share("lseek");
    std::fs::write(dir.join("seek.txt"), b"0123456789").unwrap();
    let mut proc = test_processor(&dir);
    let ino = lookup(&mut proc, 1, "seek.txt").unwrap();
    let fh = open_file(&mut proc, ino, libc::O_RDONLY as u32).unwrap();

    // SEEK_SET to offset 5
    let lseek_in = FuseLseekIn { fh, offset: 5, whence: 0, padding: 0 };
    let h = make_header(FUSE_LSEEK, ino, 1);
    let resp = proc.handle_request(&build_request(&h, fuse::as_bytes(&lseek_in)));
    assert_eq!(response_error(&resp), 0);
    let out: FuseLseekOut = fuse::read_struct(&resp[OUT_HDR_SIZE..]).unwrap();
    assert_eq!(out.offset, 5);

    // SEEK_END to offset 0 (should be at position 10)
    let lseek_in = FuseLseekIn { fh, offset: 0, whence: 2, padding: 0 };
    let h = make_header(FUSE_LSEEK, ino, 2);
    let resp = proc.handle_request(&build_request(&h, fuse::as_bytes(&lseek_in)));
    assert_eq!(response_error(&resp), 0);
    let out: FuseLseekOut = fuse::read_struct(&resp[OUT_HDR_SIZE..]).unwrap();
    assert_eq!(out.offset, 10);
}

#[test]
fn lseek_invalid_whence() {
    let dir = temp_share("lseek-bad");
    std::fs::write(dir.join("f.txt"), b"x").unwrap();
    let mut proc = test_processor(&dir);
    let ino = lookup(&mut proc, 1, "f.txt").unwrap();
    let fh = open_file(&mut proc, ino, libc::O_RDONLY as u32).unwrap();

    let lseek_in = FuseLseekIn { fh, offset: 0, whence: 99, padding: 0 };
    let h = make_header(FUSE_LSEEK, ino, 1);
    let resp = proc.handle_request(&build_request(&h, fuse::as_bytes(&lseek_in)));
    assert_eq!(response_error(&resp), -libc::EINVAL);
}

#[test]
fn read_bad_handle() {
    let dir = temp_share("read-bad-fh");
    let mut proc = test_processor(&dir);
    let read_in = FuseReadIn {
        fh: 99999, offset: 0, size: 100, read_flags: 0,
        lock_owner: 0, flags: 0, padding: 0,
    };
    let h = make_header(FUSE_READ, 1, 1);
    let resp = proc.handle_request(&build_request(&h, fuse::as_bytes(&read_in)));
    assert_eq!(response_error(&resp), -libc::EBADF);
}

// ── ops_dir tests ────────────────────────────────────────────────

#[test]
fn opendir_readdir_releasedir() {
    let dir = temp_share("dir-lifecycle");
    std::fs::write(dir.join("a.txt"), b"a").unwrap();
    std::fs::write(dir.join("b.txt"), b"b").unwrap();
    let mut proc = test_processor(&dir);

    // OPENDIR on root
    let h = make_header(FUSE_OPENDIR, 1, 1);
    let resp = proc.handle_request(&build_request(&h, &[]));
    assert_eq!(response_error(&resp), 0);
    let open_out: FuseOpenOut = fuse::read_struct(&resp[OUT_HDR_SIZE..]).unwrap();
    let fh = open_out.fh;

    // READDIR
    let read_in = FuseReadIn {
        fh, offset: 0, size: 4096, read_flags: 0,
        lock_owner: 0, flags: 0, padding: 0,
    };
    let h = make_header(FUSE_READDIR, 1, 2);
    let resp = proc.handle_request(&build_request(&h, fuse::as_bytes(&read_in)));
    assert_eq!(response_error(&resp), 0);
    // Should have data (. + .. + a.txt + b.txt = 4 entries)
    assert!(resp.len() > OUT_HDR_SIZE);

    // RELEASEDIR
    let release_in = FuseReleaseIn { fh, flags: 0, release_flags: 0, lock_owner: 0 };
    let h = make_header(FUSE_RELEASEDIR, 1, 3);
    let resp = proc.handle_request(&build_request(&h, fuse::as_bytes(&release_in)));
    assert_eq!(response_error(&resp), 0);
}

#[test]
fn readdir_includes_dot_dotdot() {
    let dir = temp_share("readdir-dots");
    let mut proc = test_processor(&dir);

    // OPENDIR
    let h = make_header(FUSE_OPENDIR, 1, 1);
    let resp = proc.handle_request(&build_request(&h, &[]));
    let open_out: FuseOpenOut = fuse::read_struct(&resp[OUT_HDR_SIZE..]).unwrap();
    let fh = open_out.fh;

    // READDIR
    let read_in = FuseReadIn {
        fh, offset: 0, size: 4096, read_flags: 0,
        lock_owner: 0, flags: 0, padding: 0,
    };
    let h = make_header(FUSE_READDIR, 1, 2);
    let resp = proc.handle_request(&build_request(&h, fuse::as_bytes(&read_in)));
    let body = &resp[OUT_HDR_SIZE..];

    // Parse first two dirents -- should be "." and ".."
    let dirent_size = std::mem::size_of::<FuseDirent>();
    let d1: FuseDirent = fuse::read_struct(body).unwrap();
    let name1 = &body[dirent_size..dirent_size + d1.namelen as usize];
    assert_eq!(name1, b".");

    let entry1_size = fuse::dirent_align(dirent_size + d1.namelen as usize);
    let d2: FuseDirent = fuse::read_struct(&body[entry1_size..]).unwrap();
    let name2 = &body[entry1_size + dirent_size..entry1_size + dirent_size + d2.namelen as usize];
    assert_eq!(name2, b"..");
}

#[test]
fn opendir_nonexistent() {
    let dir = temp_share("opendir-bad");
    let mut proc = test_processor(&dir);
    let h = make_header(FUSE_OPENDIR, 99999, 1);
    let resp = proc.handle_request(&build_request(&h, &[]));
    assert_eq!(response_error(&resp), -libc::ENOENT);
}

#[test]
fn mkdir_creates_directory() {
    let dir = temp_share("mkdir");
    let mut proc = test_processor(&dir);

    let mkdir_in = FuseMkdirIn { mode: 0o755, umask: 0 };
    let h = make_header(FUSE_MKDIR, 1, 1);
    let mut body = fuse::as_bytes(&mkdir_in).to_vec();
    body.extend_from_slice(b"subdir\0");
    let resp = proc.handle_request(&build_request(&h, &body));
    assert_eq!(response_error(&resp), 0);

    assert!(dir.join("subdir").is_dir());
    let entry: FuseEntryOut = fuse::read_struct(&resp[OUT_HDR_SIZE..]).unwrap();
    assert!(entry.nodeid > 0);
}

#[test]
fn mkdir_readonly_rejected() {
    let dir = temp_share("mkdir-ro");
    let mut proc = test_processor(&dir);
    proc.read_only = true;

    let mkdir_in = FuseMkdirIn { mode: 0o755, umask: 0 };
    let h = make_header(FUSE_MKDIR, 1, 1);
    let mut body = fuse::as_bytes(&mkdir_in).to_vec();
    body.extend_from_slice(b"nope\0");
    let resp = proc.handle_request(&build_request(&h, &body));
    assert_eq!(response_error(&resp), -libc::EROFS);
    assert!(!dir.join("nope").exists());
}

#[test]
fn unlink_removes_file() {
    let dir = temp_share("unlink");
    std::fs::write(dir.join("doomed.txt"), b"bye").unwrap();
    let mut proc = test_processor(&dir);

    let h = make_header(FUSE_UNLINK, 1, 1);
    let resp = proc.handle_request(&build_request(&h, b"doomed.txt\0"));
    assert_eq!(response_error(&resp), 0);
    assert!(!dir.join("doomed.txt").exists());
}

#[test]
fn unlink_nonexistent() {
    let dir = temp_share("unlink-none");
    let mut proc = test_processor(&dir);
    let h = make_header(FUSE_UNLINK, 1, 1);
    let resp = proc.handle_request(&build_request(&h, b"nope.txt\0"));
    assert_ne!(response_error(&resp), 0);
}

#[test]
fn unlink_readonly_rejected() {
    let dir = temp_share("unlink-ro");
    std::fs::write(dir.join("f.txt"), b"x").unwrap();
    let mut proc = test_processor(&dir);
    proc.read_only = true;

    let h = make_header(FUSE_UNLINK, 1, 1);
    let resp = proc.handle_request(&build_request(&h, b"f.txt\0"));
    assert_eq!(response_error(&resp), -libc::EROFS);
    assert!(dir.join("f.txt").exists());
}

#[test]
fn rmdir_removes_directory() {
    let dir = temp_share("rmdir");
    std::fs::create_dir(dir.join("empty_dir")).unwrap();
    let mut proc = test_processor(&dir);

    let h = make_header(FUSE_RMDIR, 1, 1);
    let resp = proc.handle_request(&build_request(&h, b"empty_dir\0"));
    assert_eq!(response_error(&resp), 0);
    assert!(!dir.join("empty_dir").exists());
}

#[test]
fn rmdir_readonly_rejected() {
    let dir = temp_share("rmdir-ro");
    std::fs::create_dir(dir.join("d")).unwrap();
    let mut proc = test_processor(&dir);
    proc.read_only = true;

    let h = make_header(FUSE_RMDIR, 1, 1);
    let resp = proc.handle_request(&build_request(&h, b"d\0"));
    assert_eq!(response_error(&resp), -libc::EROFS);
    assert!(dir.join("d").exists());
}

#[test]
fn rename_file() {
    let dir = temp_share("rename");
    std::fs::write(dir.join("old.txt"), b"content").unwrap();
    let mut proc = test_processor(&dir);

    // RENAME: old.txt -> new.txt (both in root, nodeid=1)
    let rename_in = FuseRenameIn { newdir: 1 };
    let h = make_header(FUSE_RENAME, 1, 1);
    let mut body = fuse::as_bytes(&rename_in).to_vec();
    body.extend_from_slice(b"old.txt\0new.txt\0");
    let resp = proc.handle_request(&build_request(&h, &body));
    assert_eq!(response_error(&resp), 0);

    assert!(!dir.join("old.txt").exists());
    assert_eq!(std::fs::read(dir.join("new.txt")).unwrap(), b"content");
}

#[test]
fn rename_readonly_rejected() {
    let dir = temp_share("rename-ro");
    std::fs::write(dir.join("a.txt"), b"x").unwrap();
    let mut proc = test_processor(&dir);
    proc.read_only = true;

    let rename_in = FuseRenameIn { newdir: 1 };
    let h = make_header(FUSE_RENAME, 1, 1);
    let mut body = fuse::as_bytes(&rename_in).to_vec();
    body.extend_from_slice(b"a.txt\0b.txt\0");
    let resp = proc.handle_request(&build_request(&h, &body));
    assert_eq!(response_error(&resp), -libc::EROFS);
    assert!(dir.join("a.txt").exists());
}

#[test]
fn symlink_and_readlink() {
    let dir = temp_share("symlink");
    std::fs::write(dir.join("target.txt"), b"real").unwrap();
    let mut proc = test_processor(&dir);

    // SYMLINK: create "link.txt" -> "target.txt"
    let h = make_header(FUSE_SYMLINK, 1, 1);
    let resp = proc.handle_request(&build_request(&h, b"link.txt\0target.txt\0"));
    assert_eq!(response_error(&resp), 0);
    let entry: FuseEntryOut = fuse::read_struct(&resp[OUT_HDR_SIZE..]).unwrap();
    let link_ino = entry.nodeid;

    // READLINK
    let h = make_header(FUSE_READLINK, link_ino, 2);
    let resp = proc.handle_request(&build_request(&h, &[]));
    assert_eq!(response_error(&resp), 0);
    assert_eq!(&resp[OUT_HDR_SIZE..], b"target.txt");
}

#[test]
fn symlink_readonly_rejected() {
    let dir = temp_share("symlink-ro");
    let mut proc = test_processor(&dir);
    proc.read_only = true;

    let h = make_header(FUSE_SYMLINK, 1, 1);
    let resp = proc.handle_request(&build_request(&h, b"link\0target\0"));
    assert_eq!(response_error(&resp), -libc::EROFS);
}

#[test]
fn link_creates_hardlink() {
    let dir = temp_share("hardlink");
    std::fs::write(dir.join("original.txt"), b"shared").unwrap();
    let mut proc = test_processor(&dir);
    let orig_ino = lookup(&mut proc, 1, "original.txt").unwrap();

    // LINK: create "linked.txt" pointing to original.txt's inode
    let link_in = FuseLinkIn { oldnodeid: orig_ino };
    let h = make_header(FUSE_LINK, 1, 1);
    let mut body = fuse::as_bytes(&link_in).to_vec();
    body.extend_from_slice(b"linked.txt\0");
    let resp = proc.handle_request(&build_request(&h, &body));
    assert_eq!(response_error(&resp), 0);

    // Both files should exist with same content
    assert_eq!(std::fs::read(dir.join("original.txt")).unwrap(), b"shared");
    assert_eq!(std::fs::read(dir.join("linked.txt")).unwrap(), b"shared");
}

#[test]
fn link_readonly_rejected() {
    let dir = temp_share("link-ro");
    std::fs::write(dir.join("f.txt"), b"x").unwrap();
    let mut proc = test_processor(&dir);
    let ino = lookup(&mut proc, 1, "f.txt").unwrap();
    proc.read_only = true;

    let link_in = FuseLinkIn { oldnodeid: ino };
    let h = make_header(FUSE_LINK, 1, 1);
    let mut body = fuse::as_bytes(&link_in).to_vec();
    body.extend_from_slice(b"linked.txt\0");
    let resp = proc.handle_request(&build_request(&h, &body));
    assert_eq!(response_error(&resp), -libc::EROFS);
}

#[test]
fn fsyncdir_success() {
    let dir = temp_share("fsyncdir");
    let mut proc = test_processor(&dir);

    // OPENDIR first to get a valid dir handle
    let h = make_header(FUSE_OPENDIR, 1, 1);
    let resp = proc.handle_request(&build_request(&h, &[]));
    let open_out: FuseOpenOut = fuse::read_struct(&resp[OUT_HDR_SIZE..]).unwrap();
    let fh = open_out.fh;

    // FSYNCDIR
    let fsync_in = FuseFsyncIn { fh, fsync_flags: 0, padding: 0 };
    let h = make_header(FUSE_FSYNCDIR, 1, 2);
    let resp = proc.handle_request(&build_request(&h, fuse::as_bytes(&fsync_in)));
    assert_eq!(response_error(&resp), 0);
}

#[test]
fn fsyncdir_bad_handle() {
    let dir = temp_share("fsyncdir-bad");
    let mut proc = test_processor(&dir);

    let fsync_in = FuseFsyncIn { fh: 99999, fsync_flags: 0, padding: 0 };
    let h = make_header(FUSE_FSYNCDIR, 1, 1);
    let resp = proc.handle_request(&build_request(&h, fuse::as_bytes(&fsync_in)));
    assert_eq!(response_error(&resp), -libc::EBADF);
}

// ── adversarial tests ────────────────────────────────────────────

#[test]
fn create_path_traversal_rejected() {
    let dir = temp_share("path-traversal");
    let mut proc = test_processor(&dir);

    // Try to create a file with "../" in the name
    let create_in = FuseCreateIn {
        flags: libc::O_RDWR as u32, mode: 0o644, umask: 0, open_flags: 0,
    };
    let h = make_header(FUSE_CREATE, 1, 1);
    let mut body = fuse::as_bytes(&create_in).to_vec();
    body.extend_from_slice(b"../escape.txt\0");
    let resp = proc.handle_request(&build_request(&h, &body));
    // The inode table should reject path traversal
    let err = response_error(&resp);
    assert_ne!(err, 0, "path traversal should be rejected");

    // Verify no file was created outside the share
    let parent = dir.parent().unwrap();
    assert!(!parent.join("escape.txt").exists(),
        "file must not escape the shared directory");
}

#[test]
fn unsupported_opcode_returns_enosys() {
    let dir = temp_share("enosys");
    let mut proc = test_processor(&dir);
    let h = make_header(255, 1, 1); // bogus opcode
    let resp = proc.handle_request(&build_request(&h, &[]));
    assert_eq!(response_error(&resp), -libc::ENOSYS);
}

#[test]
fn truncated_request_returns_error() {
    let dir = temp_share("truncated");
    let mut proc = test_processor(&dir);
    // Send a valid header for OPEN but with a truncated body
    let h = make_header(FUSE_OPEN, 1, 1);
    let resp = proc.handle_request(&build_request(&h, &[0])); // body too short for FuseOpenIn
    assert_ne!(response_error(&resp), 0);
}
