use super::*;

#[test]
fn changed_workload_controller_forces_one_cold_resume() {
    let session = tempfile::tempdir().unwrap();
    let checkpoint = session.path().join("checkpoint.vzsave");
    std::fs::write(&checkpoint, b"checkpoint").unwrap();
    std::fs::write(checkpoint.with_extension("vzsave.complete"), b"ok\n").unwrap();
    let entry = PersistentVmEntry {
        suspended: true,
        checkpoint_path: Some("checkpoint.vzsave".into()),
        ..crate::tests::test_persistent_entry("resume-owner", session.path().to_path_buf())
    };

    let mut current = tokio::process::Command::new("capsem-process");
    assert!(configure_resume_checkpoint(&mut current, &entry, false));
    assert!(current
        .as_std()
        .get_args()
        .any(|argument| argument == "--checkpoint-path"));

    let mut upgraded = tokio::process::Command::new("capsem-process");
    assert!(!configure_resume_checkpoint(&mut upgraded, &entry, true));
    assert!(!upgraded
        .as_std()
        .get_args()
        .any(|argument| argument == "--checkpoint-path"));
}
