use std::io::{Seek, SeekFrom, Write};
use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};

use super::*;

struct Dirs {
    _tmp: tempfile::TempDir,
    src: std::path::PathBuf,
    dst: ContainedDir,
    dst_path: std::path::PathBuf,
}

fn dirs() -> Dirs {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    let dst_path = tmp.path().join("dst");
    std::fs::create_dir(&src).unwrap();
    std::fs::create_dir(&dst_path).unwrap();
    let dst = ContainedDir::open_root(&dst_path).unwrap();
    Dirs {
        _tmp: tmp,
        src,
        dst,
        dst_path,
    }
}

fn sparse_image(path: &std::path::Path, len: u64, marker_at: u64) {
    let mut file = File::create(path).unwrap();
    file.set_len(len).unwrap();
    file.seek(SeekFrom::Start(marker_at)).unwrap();
    file.write_all(b"superblock").unwrap();
}

#[test]
fn clone_file_into_copies_content_and_sets_mode() {
    let dirs = dirs();
    std::fs::write(dirs.src.join("a"), b"hello").unwrap();
    let source = File::open(dirs.src.join("a")).unwrap();

    let (clone, _method) = clone_file_into(&source, &dirs.dst, OsStr::new("a"), 0o640).unwrap();

    drop(clone);
    assert_eq!(std::fs::read(dirs.dst_path.join("a")).unwrap(), b"hello");
    let mode = std::fs::metadata(dirs.dst_path.join("a")).unwrap().permissions().mode() & 0o7777;
    assert_eq!(mode, 0o640);
}

#[cfg(target_os = "macos")]
#[test]
fn apfs_shares_extents() {
    // Test temp directories live on APFS on every supported Mac.
    let dirs = dirs();
    std::fs::write(dirs.src.join("a"), b"hello").unwrap();
    let source = File::open(dirs.src.join("a")).unwrap();
    let (_, method) = clone_file_into(&source, &dirs.dst, OsStr::new("a"), 0o600).unwrap();
    assert_eq!(method, CloneMethod::SharedExtents);
}

#[test]
fn clone_file_into_never_replaces_or_writes_through_an_existing_entry() {
    let dirs = dirs();
    std::fs::write(dirs.src.join("a"), b"guest").unwrap();
    let outside = dirs.dst_path.parent().unwrap().join("host-file");
    std::fs::write(&outside, b"host").unwrap();
    symlink(&outside, dirs.dst_path.join("planted")).unwrap();
    symlink("dangling", dirs.dst_path.join("dangling")).unwrap();
    std::fs::write(dirs.dst_path.join("existing"), b"keep").unwrap();
    let source = File::open(dirs.src.join("a")).unwrap();

    for name in ["planted", "dangling", "existing"] {
        let error = clone_file_into(&source, &dirs.dst, OsStr::new(name), 0o600).unwrap_err();
        // EEXIST, or ELOOP where O_NOFOLLOW is checked before O_EXCL (macOS,
        // dangling link): both refuse without touching the link's target.
        assert!(
            error.kind() == io::ErrorKind::AlreadyExists || super::super::super::contained::is_symlink_refusal(&error),
            "{name}: {error}"
        );
    }
    assert_eq!(std::fs::read(&outside).unwrap(), b"host");
    assert_eq!(std::fs::read(dirs.dst_path.join("existing")).unwrap(), b"keep");
    assert!(!dirs.dst_path.parent().unwrap().join("dangling").exists());
}

#[test]
fn copy_sparse_writes_only_non_zero_blocks() {
    let dirs = dirs();
    let len = 64 * 1024 * 1024;
    sparse_image(&dirs.src.join("img"), len, len / 2);
    let source = File::open(dirs.src.join("img")).unwrap();
    let dest_path = dirs.dst_path.join("img");
    let dest = File::create(&dest_path).unwrap();

    copy_sparse(&source, &dest).unwrap();

    let meta = std::fs::metadata(&dest_path).unwrap();
    assert_eq!(meta.len(), len);
    assert!(
        meta.blocks() * 512 < 4 * 1024 * 1024,
        "one written block must not materialise a 64 MiB image: {} bytes",
        meta.blocks() * 512
    );
    let bytes = std::fs::read(&dest_path).unwrap();
    let marker = usize::try_from(len / 2).unwrap();
    assert_eq!(&bytes[marker..marker + 10], b"superblock");
    assert!(bytes[..marker].iter().all(|byte| *byte == 0));
}

#[test]
fn copy_sparse_skips_allocated_zero_blocks() {
    let dirs = dirs();
    let mut file = File::create(dirs.src.join("zeros")).unwrap();
    file.write_all(&vec![0_u8; 8 * 1024 * 1024]).unwrap();
    file.write_all(b"tail").unwrap();
    drop(file);
    let source = File::open(dirs.src.join("zeros")).unwrap();
    let dest_path = dirs.dst_path.join("zeros");
    let dest = File::create(&dest_path).unwrap();

    copy_sparse(&source, &dest).unwrap();

    let meta = std::fs::metadata(&dest_path).unwrap();
    assert_eq!(meta.len(), 8 * 1024 * 1024 + 4);
    assert!(
        meta.blocks() * 512 < 1024 * 1024,
        "{} bytes allocated",
        meta.blocks() * 512
    );
    assert!(std::fs::read(&dest_path).unwrap().ends_with(b"tail"));
}

#[test]
fn copy_sparse_handles_empty_files() {
    let dirs = dirs();
    File::create(dirs.src.join("empty")).unwrap();
    let source = File::open(dirs.src.join("empty")).unwrap();
    let dest = File::create(dirs.dst_path.join("empty")).unwrap();
    copy_sparse(&source, &dest).unwrap();
    assert_eq!(std::fs::metadata(dirs.dst_path.join("empty")).unwrap().len(), 0);
}
