use super::*;

#[test]
fn suspended_resume_preserves_its_staged_controller_across_retries() {
    let session = tempfile::tempdir().unwrap();
    let checkpoint = session.path().join("checkpoint.vzsave");
    std::fs::write(&checkpoint, b"checkpoint").unwrap();
    std::fs::write(checkpoint.with_extension("vzsave.complete"), b"ok\n").unwrap();
    let stage = session
        .path()
        .join("guest/workspace")
        .join(capsem_core::container::STAGE);
    std::fs::create_dir_all(&stage).unwrap();
    std::fs::write(stage.join("launch.py"), b"suspended launcher").unwrap();
    std::fs::write(stage.join("options.json"), b"{}").unwrap();
    std::fs::write(stage.join(capsem_core::container::STAGE_RUNNING), b"running").unwrap();
    let entry = PersistentVmEntry {
        suspended: true,
        checkpoint_path: Some("checkpoint.vzsave".into()),
        ..crate::tests::test_persistent_entry("resume-owner", session.path().to_path_buf())
    };

    for _ in 0..2 {
        let mut command = tokio::process::Command::new("capsem-process");
        assert_eq!(
            configure_resume_workload(&mut command, &entry).unwrap_err().to_string(),
            capsem_proto::WARM_SUSPEND_UNAVAILABLE
        );
        assert_eq!(command.as_std().get_args().count(), 0);
        assert_eq!(std::fs::read(stage.join("launch.py")).unwrap(), b"suspended launcher");
        assert_eq!(std::fs::read(stage.join("options.json")).unwrap(), b"{}");
        assert_eq!(
            std::fs::read(stage.join(capsem_core::container::STAGE_RUNNING)).unwrap(),
            b"running"
        );
    }
    assert_eq!(std::fs::read(&checkpoint).unwrap(), b"checkpoint");
    assert_eq!(
        std::fs::read(checkpoint.with_extension("vzsave.complete")).unwrap(),
        b"ok\n"
    );
}

#[test]
fn incomplete_suspended_checkpoints_refuse_implicit_cold_boot() {
    for (named, checkpoint_present, marker_present) in [
        (false, false, false),
        (true, false, false),
        (true, true, false),
        (true, false, true),
    ] {
        let session = tempfile::tempdir().unwrap();
        let checkpoint = session.path().join("checkpoint.vzsave");
        let complete = checkpoint_complete_path(&checkpoint);
        if checkpoint_present {
            std::fs::write(&checkpoint, b"checkpoint").unwrap();
        }
        if marker_present {
            std::fs::write(&complete, b"ok\n").unwrap();
        }
        let entry = PersistentVmEntry {
            suspended: true,
            checkpoint_path: named.then(|| "checkpoint.vzsave".into()),
            ..crate::tests::test_persistent_entry("resume-owner", session.path().to_path_buf())
        };
        let mut command = tokio::process::Command::new("capsem-process");
        let error = configure_resume_workload(&mut command, &entry).unwrap_err();
        assert!(error.to_string().contains("preserved"));
        assert_eq!(checkpoint.exists(), checkpoint_present);
        assert_eq!(complete.exists(), marker_present);
        assert_eq!(command.as_std().get_args().count(), 0);
    }
}

#[test]
fn explicitly_stopped_sessions_still_cold_boot() {
    let session = tempfile::tempdir().unwrap();
    let entry = crate::tests::test_persistent_entry("resume-owner", session.path().to_path_buf());
    assert!(!entry.suspended);
    let mut command = tokio::process::Command::new("capsem-process");
    configure_resume_workload(&mut command, &entry).unwrap();
    assert_eq!(command.as_std().get_args().count(), 0);
}
