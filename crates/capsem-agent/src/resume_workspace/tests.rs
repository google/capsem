use super::*;

#[test]
fn a_healthy_retained_workspace_is_not_unmounted() {
    let directory = tempfile::tempdir().unwrap();
    rebind_steps(
        |_| panic!("remount would reset a healthy retained VirtioFS connection"),
        || {
            enumerate(directory.path())?;
            Ok(1)
        },
    )
    .unwrap();
}

#[test]
fn a_successful_empty_directory_is_enumerable() {
    let directory = tempfile::tempdir().unwrap();
    enumerate(directory.path()).unwrap();
}

#[test]
fn a_real_nonempty_directory_is_enumerable() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("fixture"), b"synthetic").unwrap();
    enumerate(directory.path()).unwrap();
}

#[test]
fn a_failed_read_directory_remains_an_error() {
    let directory = tempfile::tempdir().unwrap();
    let missing = directory.path().join("missing");
    assert_eq!(enumerate(&missing).unwrap_err().kind(), io::ErrorKind::NotFound);
}

#[test]
fn an_iteration_error_is_not_a_warmed_directory() {
    let entries = std::iter::once(Err::<(), _>(io::Error::from_raw_os_error(5)));
    assert_eq!(first_entry(entries).unwrap_err().raw_os_error(), Some(5));
}

#[test]
fn failed_rebind_cannot_publish_ready() {
    let sent = std::cell::Cell::new(false);
    let result = ready_after(
        || Ok(()),
        || Err(io::Error::from_raw_os_error(5)),
        || {
            sent.set(true);
            Ok(())
        },
    );
    assert_eq!(result.unwrap_err().raw_os_error(), Some(5));
    assert!(!sent.get(), "Ready escaped a failed workspace repair");
}

#[test]
fn successful_resume_preparation_publishes_ready_in_order() {
    let calls = std::cell::RefCell::new(Vec::new());
    ready_after(
        || {
            calls.borrow_mut().push("thaw");
            Ok(())
        },
        || {
            calls.borrow_mut().push("rebind");
            Ok(())
        },
        || {
            calls.borrow_mut().push("ready");
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(*calls.borrow(), ["thaw", "rebind", "ready"]);
}

#[test]
fn every_required_rebind_command_failure_prevents_ready_and_later_commands() {
    for failed in 2..6 {
        let count = std::cell::Cell::new(0);
        let ready = std::cell::Cell::new(false);
        let first_probe = std::cell::Cell::new(true);
        let result = ready_after(
            || Ok(()),
            || {
                rebind_steps(
                    |_| {
                        let current = count.get();
                        count.set(current + 1);
                        if current == failed {
                            Err(io::Error::from_raw_os_error(5))
                        } else {
                            Ok(())
                        }
                    },
                    || {
                        if first_probe.replace(false) {
                            Err(io::Error::from_raw_os_error(5))
                        } else {
                            Ok(1)
                        }
                    },
                )
            },
            || {
                ready.set(true);
                Ok(())
            },
        );
        assert_eq!(result.unwrap_err().raw_os_error(), Some(5), "command {failed}");
        assert!(!ready.get(), "command {failed} failure published Ready");
        assert_eq!(count.get(), failed + 1, "commands continued after failure {failed}");
    }
}

#[test]
fn failed_enumeration_prevents_the_bind_and_ready() {
    let count = std::cell::Cell::new(0);
    let result = rebind_steps(
        |_| {
            count.set(count.get() + 1);
            Ok(())
        },
        || Err(io::Error::from_raw_os_error(5)),
    );
    assert_eq!(result.unwrap_err().raw_os_error(), Some(5));
    assert_eq!(count.get(), 4, "workspace bind followed failed enumeration");
}

#[test]
fn failed_thaw_prevents_rebind_and_ready() {
    let result = ready_after(
        || Err(io::Error::from_raw_os_error(5)),
        || panic!("rebind after failed thaw"),
        || panic!("Ready after failed thaw"),
    );
    assert_eq!(result.unwrap_err().raw_os_error(), Some(5));
}

#[test]
fn failed_ready_write_is_not_ignored() {
    assert_eq!(
        ready_after(|| Ok(()), || Ok(()), || Err(io::Error::from_raw_os_error(32)))
            .unwrap_err()
            .raw_os_error(),
        Some(32)
    );
}

#[test]
fn enumeration_retries_retain_the_original_finite_budget_and_last_error() {
    let count = std::cell::Cell::new(0);
    let pauses = std::cell::Cell::new(0);
    let error = warm_with(
        || {
            count.set(count.get() + 1);
            Err(io::Error::from_raw_os_error(5))
        },
        || pauses.set(pauses.get() + 1),
    )
    .unwrap_err();
    assert_eq!(error.raw_os_error(), Some(5));
    assert_eq!((count.get(), pauses.get()), (50, 50));
}

#[test]
fn successful_enumeration_stops_retrying_without_an_extra_pause() {
    let count = std::cell::Cell::new(0);
    let pauses = std::cell::Cell::new(0);
    assert_eq!(
        warm_with(
            || {
                count.set(count.get() + 1);
                if count.get() == 3 {
                    Ok(())
                } else {
                    Err(io::Error::from_raw_os_error(5))
                }
            },
            || pauses.set(pauses.get() + 1)
        )
        .unwrap(),
        3
    );
    assert_eq!((count.get(), pauses.get()), (3, 2));
}

#[test]
fn absent_optional_stale_mounts_do_not_hide_required_mount_failures() {
    let commands = std::cell::RefCell::new(Vec::new());
    let first_probe = std::cell::Cell::new(true);
    rebind_steps(
        |arguments| {
            commands.borrow_mut().push(arguments.join(" "));
            if arguments[0] == "umount" {
                Err(io::Error::from_raw_os_error(2))
            } else {
                Ok(())
            }
        },
        || {
            if first_probe.replace(false) {
                Err(io::Error::from_raw_os_error(5))
            } else {
                Ok(1)
            }
        },
    )
    .unwrap();
    assert_eq!(
        *commands.borrow(),
        [
            "umount -l /root",
            "umount -l /mnt/shared",
            "mkdir -p /mnt/shared",
            "mount -t virtiofs capsem /mnt/shared",
            "mkdir -p /root",
            "mount --bind /mnt/shared/workspace /root"
        ]
    );
}

#[test]
fn actual_command_spawn_and_exit_failures_are_typed_errors() {
    let directory = tempfile::tempdir().unwrap();
    let missing = directory.path().join("missing-command");
    assert_eq!(
        run(&[missing.to_str().unwrap()]).unwrap_err().kind(),
        io::ErrorKind::NotFound
    );
    assert_eq!(
        run(&["/bin/sh", "-c", "exit 7"]).unwrap_err().kind(),
        io::ErrorKind::Other
    );
}
