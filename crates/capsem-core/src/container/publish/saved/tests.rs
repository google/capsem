use super::*;

#[test]
fn saved_ports_reject_malformed_duplicate_zero_and_oversized_records() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ports");
    assert!(read(&path).unwrap().is_empty());
    for body in [
        "no-json",
        "[{}]",
        "[{\"host\":0,\"guest\":1}]",
        "[{\"host\":1,\"guest\":0}]",
        "[{\"host\":1,\"guest\":1},{\"host\":1,\"guest\":2}]",
    ] {
        std::fs::write(&path, body).unwrap();
        assert!(read(&path).is_err());
    }
    std::fs::write(&path, vec![b' '; 4097]).unwrap();
    assert!(read(&path).is_err());
    std::fs::remove_file(&path).unwrap();
    let target = dir.path().join("target");
    std::fs::write(&target, "[]").unwrap();
    std::os::unix::fs::symlink(&target, &path).unwrap();
    assert!(read(&path).is_err());
}

#[test]
fn saved_records_keep_their_target_and_old_records_are_the_containers() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ports");
    std::fs::write(
        &path,
        r#"[{"host":16379,"guest":6379},{"host":18080,"guest":8080,"target":"vm"}]"#,
    )
    .unwrap();
    let saved = read(&path).unwrap();
    assert_eq!(saved[0].target, PublicationTarget::Container);
    assert_eq!(saved[1].target, PublicationTarget::Vm);
    std::fs::write(&path, r#"[{"host":11053,"guest":1053,"target":"vm"}]"#).unwrap();
    assert!(
        read(&path).is_err(),
        "a saved VM publication of a Capsem service port must not restore"
    );
}

/// A revoked publication used to vanish only from memory: the next owner start
/// restored it from this record, reopening a port the user had closed.
#[test]
fn forgetting_a_publication_removes_it_from_what_restore_reads() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ports");
    std::fs::write(
        &path,
        r#"[{"host":16379,"guest":6379},{"host":18080,"guest":8080,"target":"vm"}]"#,
    )
    .unwrap();
    assert!(forget(&path, 16379).unwrap());
    let remaining = read(&path).unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!((remaining[0].host, remaining[0].target), (18080, PublicationTarget::Vm));
    assert!(!forget(&path, 16379).unwrap(), "forgetting twice finds nothing");
    assert!(!forget(&dir.path().join("absent"), 1).unwrap());
}
