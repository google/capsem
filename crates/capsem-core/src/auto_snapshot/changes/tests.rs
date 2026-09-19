use super::*;

#[test]
fn compares_contents_and_symlinks_against_one_baseline_in_path_order() {
    let root = tempfile::tempdir().unwrap();
    let before = root.path().join("before");
    let current = root.path().join("current");
    std::fs::create_dir_all(&before).unwrap();
    std::fs::create_dir_all(&current).unwrap();
    for dir in [&before, &current] {
        std::fs::write(dir.join("same"), "unchanged").unwrap();
        std::fs::write(dir.join("modified"), "old").unwrap();
    }
    std::fs::write(before.join("deleted"), "gone").unwrap();
    std::fs::write(current.join("created"), "new").unwrap();
    std::fs::write(current.join("modified"), "new").unwrap();
    std::os::unix::fs::symlink("/host/old", before.join("link")).unwrap();
    std::os::unix::fs::symlink("/host/new", current.join("link")).unwrap();
    let changes = workspace_changes(&before, &current).unwrap();
    assert_eq!(
        changes
            .iter()
            .map(|change| (change.path.as_str(), change.kind))
            .collect::<Vec<_>>(),
        vec![
            ("created", ChangeKind::Created),
            ("deleted", ChangeKind::Deleted),
            ("link", ChangeKind::Modified),
            ("modified", ChangeKind::Modified)
        ]
    );
    assert!(changes[2].is_symlink);
    assert_eq!(changes[1].size, None);
}

#[test]
fn comparison_never_descends_symlinks_or_blocks_on_special_files() {
    let root = tempfile::tempdir().unwrap();
    let before = root.path().join("before");
    let current = root.path().join("current");
    let outside = root.path().join("outside");
    for dir in [&before, &current, &outside] {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(outside.join("secret"), "outside contents").unwrap();
    std::os::unix::fs::symlink(&outside, current.join("escape")).unwrap();
    nix::unistd::mkfifo(&current.join("pipe"), nix::sys::stat::Mode::S_IRUSR).unwrap();
    let changes = workspace_changes(&before, &current).unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].path, "escape");
    assert!(changes[0].is_symlink);
}

#[test]
fn missing_baseline_is_an_error_not_an_empty_checkpoint() {
    let root = tempfile::tempdir().unwrap();
    assert!(workspace_changes(&root.path().join("absent"), root.path()).is_err());
}

/// A checkpoint whose manifest recorded the live tree before cloning it.
fn checkpointed(root: &std::path::Path) -> (std::path::PathBuf, std::path::PathBuf, WorkspaceManifest) {
    let live = root.join("live");
    let slot = root.join("slot");
    std::fs::create_dir_all(live.join("dir")).unwrap();
    std::fs::create_dir_all(&slot).unwrap();
    std::fs::write(live.join("dir/nested"), "nested contents").unwrap();
    std::os::unix::fs::symlink("/host/target", live.join("link")).unwrap();
    // `same` is written last, a tick after the rest, so a test can make it --
    // and only it -- not strictly older than the capture clock.
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(live.join("same"), "unchanged contents").unwrap();
    // Past the timestamp granularity, so the capture clock is strictly later
    // than every entry and none is treated as racily clean.
    std::thread::sleep(std::time::Duration::from_millis(20));
    let manifest = WorkspaceManifest::capture(&live, &slot).unwrap();
    let checkpoint = slot.join("workspace");
    copy_tree(&live, &checkpoint);
    (live, checkpoint, manifest)
}

fn copy_tree(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let kind = entry.file_type().unwrap();
        let target = to.join(entry.file_name());
        if kind.is_dir() {
            copy_tree(&entry.path(), &target);
        } else if kind.is_symlink() {
            std::os::unix::fs::symlink(std::fs::read_link(entry.path()).unwrap(), target).unwrap();
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn counted(
    checkpoint: &std::path::Path,
    live: &std::path::Path,
    manifest: Option<&WorkspaceManifest>,
) -> (Vec<(String, ChangeKind)>, usize) {
    let mut reads = 0;
    let changes = diff(checkpoint, live, manifest, &mut |file| {
        reads += 1;
        digest_file(file)
    })
    .unwrap();
    (
        changes.into_iter().map(|change| (change.path, change.kind)).collect(),
        reads,
    )
}

/// Finding 25: listing changes read every file of both trees on every page.
#[test]
fn an_unchanged_workspace_is_compared_without_reading_a_file() {
    let root = tempfile::tempdir().unwrap();
    let (live, checkpoint, manifest) = checkpointed(root.path());
    let (changes, reads) = counted(&checkpoint, &live, Some(&manifest));
    assert!(changes.is_empty(), "{changes:?}");
    assert_eq!(
        reads, 0,
        "identities recorded at checkpoint time answer without reading"
    );
}

/// The guest controls mtime. An edit that keeps the size and puts mtime back
/// must still be seen, or the change list is one an agent can edit.
#[test]
fn a_same_size_edit_with_mtime_restored_is_still_reported() {
    let root = tempfile::tempdir().unwrap();
    let (live, checkpoint, manifest) = checkpointed(root.path());
    let mtime = std::fs::metadata(live.join("same")).unwrap().modified().unwrap();
    std::fs::write(live.join("same"), "tampered contents!").unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .open(live.join("same"))
        .unwrap()
        .set_modified(mtime)
        .unwrap();

    let (changes, reads) = counted(&checkpoint, &live, Some(&manifest));
    assert_eq!(changes, vec![("same".to_string(), ChangeKind::Modified)]);
    assert_eq!(reads, 2, "only the entry whose identity moved is read, on both sides");
}

/// An entry changed in the same timestamp tick as the capture can keep its
/// recorded ctime (the racy-git problem), so it is never trusted.
#[test]
fn an_entry_not_strictly_older_than_the_capture_clock_is_verified() {
    let root = tempfile::tempdir().unwrap();
    let (live, checkpoint, mut manifest) = checkpointed(root.path());
    // Pull the capture clock back to the entry's own ctime: it was not
    // strictly older than the capture, so its identity proves nothing.
    manifest.clock = manifest.entries["same"].ctime;
    let (changes, reads) = counted(&checkpoint, &live, Some(&manifest));
    assert!(changes.is_empty(), "{changes:?}");
    assert_eq!(reads, 2, "the racy entry is verified by content; the rest are trusted");
}

/// Without a manifest (checkpoints taken before one was recorded) the
/// comparison stays exact: only same-size pairs need reading.
#[test]
fn without_a_manifest_only_same_size_pairs_are_read() {
    let root = tempfile::tempdir().unwrap();
    let (live, checkpoint, _) = checkpointed(root.path());
    std::fs::write(live.join("dir/nested"), "grown well past its old size").unwrap();
    let (changes, reads) = counted(&checkpoint, &live, None);
    assert_eq!(changes, vec![("dir/nested".to_string(), ChangeKind::Modified)]);
    assert_eq!(reads, 2, "`same` is read on both sides; a size change needs no read");
}

/// Through the real scheduler: `take_snapshot` records the manifest before it
/// clones, and `changes_since` uses it -- answering an unchanged workspace
/// without reads, yet still seeing a same-size edit with mtime restored.
#[test]
fn a_taken_snapshot_records_the_manifest_that_changes_since_uses() {
    let tmp = tempfile::tempdir().unwrap();
    let session = tmp.path();
    for dir in ["workspace", "system", "auto_snapshots"] {
        std::fs::create_dir_all(session.join(dir)).unwrap();
    }
    let live = session.join("workspace");
    std::fs::write(live.join("file"), "original").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    let mut scheduler = crate::auto_snapshot::AutoSnapshotScheduler::new(
        session.to_path_buf(),
        3,
        4,
        std::time::Duration::from_secs(300),
    );
    let slot = scheduler.take_snapshot().unwrap();

    let manifest = WorkspaceManifest::load(slot.workspace_path.parent().unwrap());
    assert!(manifest.is_some(), "take_snapshot records the manifest");
    assert!(changes_since(&slot, &live).unwrap().is_empty());

    let mtime = std::fs::metadata(live.join("file")).unwrap().modified().unwrap();
    std::fs::write(live.join("file"), "tampered").unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .open(live.join("file"))
        .unwrap()
        .set_modified(mtime)
        .unwrap();
    let changes = changes_since(&slot, &live).unwrap();
    assert_eq!(
        changes
            .iter()
            .map(|change| (change.path.as_str(), change.kind))
            .collect::<Vec<_>>(),
        vec![("file", ChangeKind::Modified)]
    );
}
