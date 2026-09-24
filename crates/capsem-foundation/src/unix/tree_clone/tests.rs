use std::ffi::OsStr;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::Path;

use super::*;

struct Fixture {
    _tmp: tempfile::TempDir,
    src: std::path::PathBuf,
    dst: std::path::PathBuf,
    /// A host file outside both trees that no clone may ever read.
    secret: std::path::PathBuf,
}

fn fixture() -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    let dst = tmp.path().join("dst");
    std::fs::create_dir(&src).unwrap();
    std::fs::create_dir(&dst).unwrap();
    let secret = tmp.path().join("host-secret");
    std::fs::write(&secret, b"host secret").unwrap();
    Fixture {
        _tmp: tmp,
        src,
        dst,
        secret,
    }
}

fn clone(fixture: &Fixture) -> CloneStats {
    let src = ContainedDir::open_root(&fixture.src).unwrap();
    let dst = ContainedDir::open_root(&fixture.dst).unwrap();
    clone_tree(&src, &dst).unwrap()
}

fn mode(path: &Path) -> u32 {
    std::fs::symlink_metadata(path).unwrap().permissions().mode() & 0o7777
}

#[test]
fn clones_nested_content_and_permissions() {
    let fixture = fixture();
    std::fs::create_dir_all(fixture.src.join("a/b")).unwrap();
    std::fs::write(fixture.src.join("top.txt"), b"top").unwrap();
    std::fs::write(fixture.src.join("a/b/deep.bin"), [0_u8, 1, 2, 255]).unwrap();
    std::fs::write(fixture.src.join("empty"), b"").unwrap();
    std::fs::set_permissions(fixture.src.join("top.txt"), std::fs::Permissions::from_mode(0o640)).unwrap();

    let stats = clone(&fixture);

    assert_eq!(std::fs::read(fixture.dst.join("top.txt")).unwrap(), b"top");
    assert_eq!(std::fs::read(fixture.dst.join("a/b/deep.bin")).unwrap(), [0_u8, 1, 2, 255]);
    assert_eq!(std::fs::read(fixture.dst.join("empty")).unwrap(), b"");
    assert_eq!(mode(&fixture.dst.join("top.txt")), 0o640);
    assert_eq!(stats.cloned_files + stats.copied_files, 3);
    assert_eq!(stats.directories, 2);
    assert_eq!(stats.skipped, 0);
}

#[test]
fn clone_is_independent_of_its_source() {
    let fixture = fixture();
    std::fs::write(fixture.src.join("file"), b"before").unwrap();
    clone(&fixture);
    std::fs::write(fixture.src.join("file"), b"after, and longer").unwrap();
    assert_eq!(std::fs::read(fixture.dst.join("file")).unwrap(), b"before");
}

#[test]
fn symlinks_are_recreated_verbatim_and_never_followed() {
    let fixture = fixture();
    symlink(&fixture.secret, fixture.src.join("to-secret")).unwrap();
    symlink(fixture.secret.parent().unwrap(), fixture.src.join("to-host-dir")).unwrap();
    symlink("dangling-target", fixture.src.join("dangling")).unwrap();

    let stats = clone(&fixture);

    assert_eq!(std::fs::read_link(fixture.dst.join("to-secret")).unwrap(), fixture.secret);
    assert_eq!(std::fs::read_link(fixture.dst.join("dangling")).unwrap(), Path::new("dangling-target"));
    assert!(std::fs::symlink_metadata(fixture.dst.join("to-host-dir"))
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(stats.symlinks, 3);
    assert_eq!(stats.cloned_files + stats.copied_files, 0, "no link target was copied");
}

#[test]
fn a_root_that_is_a_symlink_is_refused_not_followed() {
    // The guest can replace `workspace/` itself; the caller descends into it
    // through a ContainedDir, which must refuse the link.
    let fixture = fixture();
    let host_dir = fixture.secret.parent().unwrap();
    symlink(host_dir, fixture.src.join("workspace")).unwrap();
    let src = ContainedDir::open_root(&fixture.src).unwrap();
    let error = src.descend(OsStr::new("workspace")).unwrap_err();
    assert!(is_symlink_refusal(&error), "{error}");
}

#[test]
fn a_file_swapped_for_a_symlink_after_listing_is_skipped_not_followed() {
    // Deterministic form of the live-guest race: list, then swap, then clone
    // the entry. The open must refuse the link rather than read the host file.
    let fixture = fixture();
    std::fs::write(fixture.src.join("victim"), b"guest data").unwrap();
    let src = ContainedDir::open_root(&fixture.src).unwrap();
    let dst = ContainedDir::open_root(&fixture.dst).unwrap();
    let listed = src.entries().unwrap();
    assert_eq!(listed[0].kind, EntryKind::File);

    std::fs::remove_file(fixture.src.join("victim")).unwrap();
    symlink(&fixture.secret, fixture.src.join("victim")).unwrap();

    let mut stats = CloneStats::default();
    let error = clone_entry(&src, &dst, OsStr::new("victim"), EntryKind::File, &mut stats).unwrap_err();
    assert!(is_race(&error), "{error}");
    assert!(!fixture.dst.join("victim").exists());
}

#[test]
fn a_directory_swapped_for_a_symlink_after_listing_is_skipped_not_followed() {
    let fixture = fixture();
    std::fs::create_dir(fixture.src.join("dir")).unwrap();
    let src = ContainedDir::open_root(&fixture.src).unwrap();
    let dst = ContainedDir::open_root(&fixture.dst).unwrap();

    std::fs::remove_dir(fixture.src.join("dir")).unwrap();
    symlink(fixture.secret.parent().unwrap(), fixture.src.join("dir")).unwrap();

    let mut stats = CloneStats::default();
    let error = clone_entry(&src, &dst, OsStr::new("dir"), EntryKind::Directory, &mut stats).unwrap_err();
    assert!(is_race(&error), "{error}");
    assert!(!fixture.dst.join("dir").exists());
}

#[test]
fn a_live_guest_swapping_entries_never_leaks_a_host_file() {
    // Race the walker against a thread that keeps swapping a file and a
    // directory for links to host data. Whatever interleaving happens, the
    // clone must never contain the secret's bytes.
    let fixture = fixture();
    for index in 0..64 {
        std::fs::write(fixture.src.join(format!("f{index}")), b"guest").unwrap();
    }
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let swapper = {
        let stop = std::sync::Arc::clone(&stop);
        let src = fixture.src.clone();
        let secret = fixture.secret.clone();
        std::thread::spawn(move || {
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                for index in 0..64 {
                    let path = src.join(format!("f{index}"));
                    let _ = std::fs::remove_file(&path);
                    let _ = symlink(&secret, &path);
                    let _ = std::fs::remove_file(&path);
                    let _ = std::fs::write(&path, b"guest");
                }
            }
        })
    };
    for round in 0..20 {
        let dst = fixture.dst.join(format!("round{round}"));
        std::fs::create_dir(&dst).unwrap();
        let src_dir = ContainedDir::open_root(&fixture.src).unwrap();
        let dst_dir = ContainedDir::open_root(&dst).unwrap();
        clone_tree(&src_dir, &dst_dir).unwrap();
        for entry in std::fs::read_dir(&dst).unwrap() {
            let path = entry.unwrap().path();
            let meta = std::fs::symlink_metadata(&path).unwrap();
            if meta.is_file() {
                assert_ne!(std::fs::read(&path).unwrap(), b"host secret", "{}", path.display());
            }
        }
    }
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    swapper.join().unwrap();
}

#[test]
fn special_files_are_skipped_without_blocking() {
    let fixture = fixture();
    nix::unistd::mkfifo(&fixture.src.join("fifo"), nix::sys::stat::Mode::from_bits_truncate(0o600)).unwrap();
    let stats = clone(&fixture);
    assert_eq!(stats.skipped, 1);
    assert!(std::fs::symlink_metadata(fixture.dst.join("fifo")).is_err());
}

#[test]
fn privileged_mode_bits_are_dropped() {
    let fixture = fixture();
    std::fs::write(fixture.src.join("tool"), b"#!/bin/sh\n").unwrap();
    std::fs::set_permissions(fixture.src.join("tool"), std::fs::Permissions::from_mode(0o4755)).unwrap();
    std::fs::create_dir(fixture.src.join("shared")).unwrap();
    std::fs::set_permissions(fixture.src.join("shared"), std::fs::Permissions::from_mode(0o1777)).unwrap();

    clone(&fixture);

    assert_eq!(mode(&fixture.dst.join("tool")), 0o755);
    assert_eq!(mode(&fixture.dst.join("shared")), 0o777);
}

#[test]
fn read_only_source_directories_are_still_filled_then_restored() {
    let fixture = fixture();
    std::fs::create_dir(fixture.src.join("ro")).unwrap();
    std::fs::write(fixture.src.join("ro/inside"), b"x").unwrap();
    std::fs::set_permissions(fixture.src.join("ro"), std::fs::Permissions::from_mode(0o555)).unwrap();

    clone(&fixture);

    assert_eq!(std::fs::read(fixture.dst.join("ro/inside")).unwrap(), b"x");
    assert_eq!(mode(&fixture.dst.join("ro")), 0o555);
    // Let the tempdir clean up.
    std::fs::set_permissions(fixture.dst.join("ro"), std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::set_permissions(fixture.src.join("ro"), std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn clone_file_refuses_to_replace_an_existing_destination_or_link() {
    let fixture = fixture();
    std::fs::write(fixture.src.join("image"), b"image").unwrap();
    symlink(&fixture.secret, fixture.dst.join("planted")).unwrap();
    std::fs::write(fixture.dst.join("existing"), b"keep").unwrap();
    let src = ContainedDir::open_root(&fixture.src).unwrap();
    let dst = ContainedDir::open_root(&fixture.dst).unwrap();

    for name in ["planted", "existing"] {
        let error = clone_file(&src, OsStr::new("image"), &dst, OsStr::new(name)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists, "{name}: {error}");
    }
    assert_eq!(std::fs::read(&fixture.secret).unwrap(), b"host secret");
    assert_eq!(std::fs::read(fixture.dst.join("existing")).unwrap(), b"keep");
}

#[test]
fn sync_file_refuses_a_symlink() {
    let fixture = fixture();
    symlink(&fixture.secret, fixture.src.join("rootfs.img")).unwrap();
    let src = ContainedDir::open_root(&fixture.src).unwrap();
    let error = sync_file(&src, OsStr::new("rootfs.img")).unwrap_err();
    assert!(is_symlink_refusal(&error), "{error}");
}

#[test]
fn sparse_images_stay_sparse() {
    let fixture = fixture();
    let len = 64 * 1024 * 1024;
    {
        use std::io::{Seek, SeekFrom, Write};
        let mut file = std::fs::File::create(fixture.src.join("rootfs.img")).unwrap();
        file.set_len(len).unwrap();
        file.seek(SeekFrom::Start(len / 2)).unwrap();
        file.write_all(b"superblock").unwrap();
    }
    clone(&fixture);
    let meta = std::fs::metadata(fixture.dst.join("rootfs.img")).unwrap();
    assert_eq!(meta.len(), len);
    use std::os::unix::fs::MetadataExt;
    assert!(
        meta.blocks() * 512 < 8 * 1024 * 1024,
        "a 64 MiB image with one written block must not be materialised: {} bytes allocated",
        meta.blocks() * 512
    );
}
