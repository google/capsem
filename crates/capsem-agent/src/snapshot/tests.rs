use super::*;

#[test]
fn reconnect_and_duplicate_unfreeze_do_not_touch_the_filesystem() {
    let mut frozen = false;
    transition(&mut frozen, false, |_| panic!("ordinary reconnect must not thaw")).unwrap();
    transition(&mut frozen, true, |requested| {
        assert!(requested);
        Ok(())
    })
    .unwrap();
    // This is the state captured by the host after SnapshotReady.
    assert!(frozen);
    transition(&mut frozen, false, |requested| {
        assert!(!requested);
        Ok(())
    })
    .unwrap();
    transition(&mut frozen, false, |_| panic!("duplicate Unfreeze must be harmless")).unwrap();
}

#[test]
fn failed_freeze_or_thaw_keeps_the_last_successful_state() {
    for mut frozen in [false, true] {
        let previous = frozen;
        let result = transition(&mut frozen, !previous, |_| Err(io::Error::other("fixture failure")));
        assert!(result.is_err());
        assert_eq!(frozen, previous);
        transition(&mut frozen, !previous, |_| Ok(())).unwrap();
        assert_eq!(frozen, !previous);
    }
}

#[test]
fn snapshot_freeze_commands_target_the_persistent_ext4_upper() {
    let freeze = fsfreeze_command("-f");
    assert_eq!(freeze.get_program(), "fsfreeze");
    assert_eq!(
        freeze
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>(),
        ["-f", SYSTEM_FS_MOUNT]
    );

    let thaw = fsfreeze_command("-u");
    assert_eq!(thaw.get_program(), "fsfreeze");
    assert_eq!(
        thaw.get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>(),
        ["-u", SYSTEM_FS_MOUNT]
    );
}

#[test]
fn ordinary_reconnect_does_not_thaw_an_unfrozen_filesystem() {
    thaw_system_filesystem().expect("a transport reconnect without a snapshot must keep the agent alive");
}
