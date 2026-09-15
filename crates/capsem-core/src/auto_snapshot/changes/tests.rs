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
