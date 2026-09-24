use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};
use std::sync::{Arc, Barrier};

use super::{
    atomic_write_private, create_private_sibling, durable_sync_directory, durable_sync_file, ensure_private_dir,
    filesystem_space, open_private_append_no_follow, open_regular_file_no_follow, read_regular_file_no_follow,
    rename_private_sibling, write_new_regular_file_no_follow,
};

#[test]
fn filesystem_capacity_is_internally_consistent() {
    let root = tempfile::tempdir().unwrap();
    let space = filesystem_space(root.path()).unwrap();
    assert!(space.total_bytes > 0);
    assert!(space.free_bytes <= space.total_bytes);
    assert!(space.available_bytes <= space.free_bytes);
}

#[test]
fn private_directory_is_created_owner_only() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("private");
    ensure_private_dir(&path).unwrap();

    let metadata = std::fs::symlink_metadata(path).unwrap();
    assert!(metadata.is_dir());
    assert_eq!(metadata.uid(), nix::unistd::getuid().as_raw());
    assert_eq!(metadata.permissions().mode() & 0o777, 0o700);
}

#[test]
fn private_directory_refuses_symlinks_and_permissive_modes() {
    let root = tempfile::tempdir().unwrap();
    let elsewhere = root.path().join("elsewhere");
    std::fs::create_dir(&elsewhere).unwrap();
    let link = root.path().join("link");
    symlink(&elsewhere, &link).unwrap();
    assert!(ensure_private_dir(&link).is_err());

    let permissive = root.path().join("permissive");
    std::fs::create_dir(&permissive).unwrap();
    std::fs::set_permissions(&permissive, std::fs::Permissions::from_mode(0o750)).unwrap();
    assert!(ensure_private_dir(&permissive).is_err());

    let unusable = root.path().join("unusable");
    std::fs::create_dir(&unusable).unwrap();
    std::fs::set_permissions(&unusable, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(ensure_private_dir(&unusable).is_err());
}

#[test]
fn private_atomic_write_replaces_content_at_mode_600() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("secret");
    std::fs::write(&path, b"old").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).unwrap();

    atomic_write_private(&path, b"new secret").unwrap();

    assert_eq!(std::fs::read(&path).unwrap(), b"new secret");
    assert_eq!(
        std::fs::symlink_metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn concurrent_private_writes_never_publish_partial_content() {
    let root = tempfile::tempdir().unwrap();
    let path = Arc::new(root.path().join("secret"));
    let barrier = Arc::new(Barrier::new(3));
    let first = vec![b'a'; 32 * 1024];
    let second = vec![b'b'; 48 * 1024];
    let handles = [first.clone(), second.clone()].map(|content| {
        let path = Arc::clone(&path);
        let barrier = Arc::clone(&barrier);
        std::thread::spawn(move || {
            barrier.wait();
            atomic_write_private(&path, &content).unwrap();
        })
    });
    barrier.wait();
    for handle in handles {
        handle.join().unwrap();
    }

    let published = std::fs::read(&*path).unwrap();
    assert!(published == first || published == second);
    assert_eq!(root.path().read_dir().unwrap().count(), 1);
}

#[test]
fn regular_file_helpers_refuse_symlinks_and_special_files() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("target");
    std::fs::write(&target, b"secret").unwrap();
    let link = root.path().join("link");
    symlink(&target, &link).unwrap();
    assert!(read_regular_file_no_follow(&link).is_err());
    assert!(write_new_regular_file_no_follow(&link, b"replaced", 0o600).is_err());
    assert_eq!(std::fs::read(&target).unwrap(), b"secret");

    let fifo = root.path().join("fifo");
    nix::unistd::mkfifo(&fifo, nix::sys::stat::Mode::from_bits_truncate(0o600)).unwrap();
    let error = read_regular_file_no_follow(&fifo).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
}

#[test]
fn new_regular_file_is_complete_and_uses_requested_mode() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("restored");
    write_new_regular_file_no_follow(&path, b"restored bytes", 0o640).unwrap();
    assert_eq!(read_regular_file_no_follow(&path).unwrap(), b"restored bytes");
    assert_eq!(
        std::fs::symlink_metadata(path).unwrap().permissions().mode() & 0o777,
        0o640
    );
}

#[test]
fn durability_barriers_cover_file_bytes_and_directory_names() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("generation");
    std::fs::write(&path, b"bytes").unwrap();
    let file = std::fs::File::open(&path).unwrap();
    durable_sync_file(&file).unwrap();
    durable_sync_directory(root.path()).unwrap();

    let (pipe_reader, _pipe_writer) = nix::unistd::pipe().unwrap();
    let pipe_reader = std::fs::File::from(pipe_reader);
    assert!(durable_sync_file(&pipe_reader).is_err());
    assert!(durable_sync_directory(&path).is_err());
}

#[test]
fn private_append_open_creates_an_owner_only_file() {
    use std::io::Write;

    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("append.log");
    let mut file = open_private_append_no_follow(&path).unwrap();
    file.write_all(b"first").unwrap();
    drop(file);

    assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);

    let mut reopened = open_private_append_no_follow(&path).unwrap();
    reopened.write_all(b" second").unwrap();
    drop(reopened);
    assert_eq!(std::fs::read(&path).unwrap(), b"first second");
}

/// The whole reason an append-only log can also read itself: the write cursor
/// is the end of the file, not wherever the last read left off.
#[test]
fn private_append_open_writes_at_the_end_whatever_the_read_cursor_does() {
    use std::io::{Read, Seek, SeekFrom, Write};

    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("append.log");
    std::fs::write(&path, b"already here").unwrap();

    let mut file = open_private_append_no_follow(&path).unwrap();
    file.seek(SeekFrom::Start(0)).unwrap();
    let mut head = [0u8; 7];
    file.read_exact(&mut head).unwrap();
    assert_eq!(&head, b"already");
    file.write_all(b", and more").unwrap();
    drop(file);

    assert_eq!(std::fs::read(&path).unwrap(), b"already here, and more");
}

#[test]
fn private_append_open_refuses_a_symlink_and_a_fifo() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("target");
    std::fs::write(&target, b"someone else's file").unwrap();
    let link = root.path().join("link");
    symlink(&target, &link).unwrap();

    assert!(open_private_append_no_follow(&link).is_err());
    assert_eq!(
        std::fs::read(&target).unwrap(),
        b"someone else's file",
        "the link's target is never written through"
    );

    let fifo = root.path().join("fifo");
    nix::unistd::mkfifo(&fifo, nix::sys::stat::Mode::from_bits_truncate(0o600)).unwrap();
    let error = open_private_append_no_follow(&fifo).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
}

#[test]
fn regular_file_open_refuses_a_symlink_and_a_directory() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("target");
    std::fs::write(&target, b"bytes").unwrap();
    let link = root.path().join("link");
    symlink(&target, &link).unwrap();

    assert!(open_regular_file_no_follow(&link).is_err());
    assert!(open_regular_file_no_follow(root.path()).is_err());
    assert!(open_regular_file_no_follow(&target).is_ok());
}

/// A temporary exists to become another file. Every way of not getting there
/// must take it with them, including the one nobody writes cleanup for.
#[test]
fn a_private_sibling_removes_itself_unless_it_is_renamed() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("secrets");

    let abandoned = {
        let sibling = create_private_sibling(&destination).unwrap();
        sibling.path().to_path_buf()
    };
    assert!(!abandoned.exists(), "a dropped sibling takes its file with it");

    let panicked = std::sync::Mutex::new(None);
    let result = std::panic::catch_unwind(|| {
        let sibling = create_private_sibling(&destination).unwrap();
        *panicked.lock().unwrap() = Some(sibling.path().to_path_buf());
        panic!("the caller fell over mid-write");
    });
    assert!(result.is_err());
    let panicked = panicked.lock().unwrap().clone().expect("the path was recorded");
    assert!(
        !panicked.exists(),
        "an unwind past a temporary must not leave it beside the file it was to become"
    );

    let mut sibling = create_private_sibling(&destination).unwrap();
    std::io::Write::write_all(sibling.file(), b"kept").unwrap();
    rename_private_sibling(sibling, &destination).unwrap();
    assert_eq!(std::fs::read(&destination).unwrap(), b"kept");
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().flatten().count(),
        1,
        "and nothing is left beside it"
    );
}
