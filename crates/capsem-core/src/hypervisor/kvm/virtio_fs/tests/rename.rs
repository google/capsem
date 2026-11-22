//! VirtioFS rename behavior and destination preservation.

use super::*;

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
fn rename2_noreplace_preserves_existing_directory_then_publishes_to_absent_path() {
    let dir = temp_share("rename2-noreplace");
    std::fs::create_dir(dir.join("staged")).unwrap();
    std::fs::write(dir.join("staged/contents"), b"approved").unwrap();
    std::fs::create_dir(dir.join("project")).unwrap();
    let mut proc = test_processor(&dir);
    let h = make_header(FUSE_RENAME2, 1, 1);
    let mut body = fuse::as_bytes(&FuseRename2In {
        newdir: 1,
        flags: libc::RENAME_NOREPLACE,
        padding: 0,
    })
    .to_vec();
    body.extend_from_slice(b"staged\0project\0");
    let request = build_request(&h, &body);
    assert_eq!(response_error(&proc.handle_request(&request)), -libc::EEXIST);
    assert!(dir.join("staged/contents").exists());
    assert!(std::fs::read_dir(dir.join("project")).unwrap().next().is_none());

    std::fs::remove_dir(dir.join("project")).unwrap();
    assert_eq!(response_error(&proc.handle_request(&request)), 0);
    assert!(!dir.join("staged").exists());
    assert_eq!(std::fs::read(dir.join("project/contents")).unwrap(), b"approved");
}

#[test]
fn rename_over_existing_rebinds_source_inode_to_target_path() {
    let dir = temp_share("rename-over-existing");
    std::fs::write(dir.join("config.json"), b"old").unwrap();
    std::fs::write(dir.join("config.json.tmp"), b"new").unwrap();
    let mut proc = test_processor(&dir);
    let _target_ino = lookup(&mut proc, 1, "config.json").unwrap();
    let temp_ino = lookup(&mut proc, 1, "config.json.tmp").unwrap();

    let rename_in = FuseRenameIn { newdir: 1 };
    let h = make_header(FUSE_RENAME, 1, 1);
    let mut body = fuse::as_bytes(&rename_in).to_vec();
    body.extend_from_slice(b"config.json.tmp\0config.json\0");
    let resp = proc.handle_request(&build_request(&h, &body));
    assert_eq!(response_error(&resp), 0);

    let fh = open_file(&mut proc, temp_ino, libc::O_RDONLY as u32).unwrap();
    let read_in = FuseReadIn {
        fh,
        offset: 0,
        size: 1024,
        read_flags: 0,
        lock_owner: 0,
        flags: 0,
        padding: 0,
    };
    let h = make_header(FUSE_READ, temp_ino, 2);
    let resp = proc.handle_request(&build_request(&h, fuse::as_bytes(&read_in)));
    assert_eq!(response_error(&resp), 0);
    assert_eq!(&resp[OUT_HDR_SIZE..], b"new");
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
