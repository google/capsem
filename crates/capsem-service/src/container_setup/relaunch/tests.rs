use super::*;
use crate::tests::{insert_fake_instance_with_session_dir, make_test_state_owned};

const IMAGE: &str = "registry.example/app:1";

fn record(exposure_id: Option<&str>) -> LaunchRecord {
    LaunchRecord {
        image: IMAGE.into(),
        digest: "sha256:aa".into(),
        surface: Some(ContainerSurface {
            kind: ContainerSurfaceKind::Xpra,
            port: 14500,
            exposure_id: exposure_id.map(str::to_owned),
        }),
        resolved: Some("registry.example/app@sha256:aa".into()),
        manifest: Some(MANIFEST.into()),
        cache_key: None,
    }
}

const MANIFEST: &str = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn write_record(session_dir: &std::path::Path, record: &LaunchRecord) {
    std::fs::create_dir_all(session_dir).unwrap();
    capsem_foundation::unix::fs::atomic_write_private(
        &session_dir.join(LAUNCH_RECORD),
        &serde_json::to_vec(record).unwrap(),
    )
    .unwrap();
}

/// A clone carries the workload, so exec and the files API keep treating it
/// as an image session; the exposure stays with the source's owner.
#[test]
fn a_clone_carries_the_launch_record_without_the_sources_exposure() {
    let dir = tempfile::tempdir().unwrap();
    let (source, clone) = (dir.path().join("source"), dir.path().join("clone"));
    write_record(&source, &record(Some("exposure-of-the-source")));
    std::fs::create_dir_all(&clone).unwrap();

    carry_launch_record(&source, &clone).unwrap();

    let carried = read_launch_record(&clone).expect("carried");
    assert_eq!(carried.image, IMAGE);
    assert_eq!(carried.resolved.as_deref(), Some("registry.example/app@sha256:aa"));
    assert_eq!(carried.manifest.as_deref(), Some(MANIFEST));
    assert_eq!(carried.surface.unwrap().exposure_id, None);
    assert!(read_launch_record(&source)
        .unwrap()
        .surface
        .unwrap()
        .exposure_id
        .is_some());
}

#[test]
fn a_clone_of_a_bare_vm_carries_nothing() {
    let dir = tempfile::tempdir().unwrap();
    carry_launch_record(dir.path(), dir.path()).unwrap();
    assert!(!dir.path().join(LAUNCH_RECORD).exists());
}

/// `--from` with `--image`: the clone's first boot must launch nothing, so
/// the named image is the only workload; the guest-written markers are
/// unlinked, never followed, and the rest of the workspace stays.
#[test]
fn a_clone_taking_a_new_image_forgets_the_whole_carried_stage() {
    let dir = tempfile::tempdir().unwrap();
    let session = dir.path().join("clone");
    write_record(&session, &record(None));
    let stage = session.join("guest/workspace").join(capsem_core::container::STAGE);
    std::fs::create_dir_all(&stage).unwrap();
    std::fs::write(stage.join("ready"), b"1\n").unwrap();
    std::fs::write(stage.join("index.json"), b"{}").unwrap();
    std::fs::write(stage.join("launch.py"), b"#").unwrap();
    std::fs::set_permissions(
        stage.join("launch.py"),
        std::os::unix::fs::PermissionsExt::from_mode(0o555),
    )
    .unwrap();
    std::fs::write(session.join("guest/workspace/notes.txt"), b"kept").unwrap();
    let share = capsem_core::session::prepare_image_share(&session).unwrap();
    std::fs::write(share.join(MANIFEST.trim_start_matches("sha256:")), b"{}").unwrap();
    let outside = dir.path().join("host-file");
    std::fs::write(&outside, b"keep").unwrap();
    std::os::unix::fs::symlink(&outside, stage.join("running")).unwrap();

    drop_carried_image(&session).unwrap();

    assert!(!session.join(LAUNCH_RECORD).exists());
    assert!(
        capsem_core::session::image_share_blobs(&session).unwrap().is_empty(),
        "no blob of the source's image is left for the new one"
    );
    assert!(!stage.join("ready").exists());
    assert!(std::fs::symlink_metadata(stage.join("running")).is_err());
    assert_eq!(std::fs::read(&outside).unwrap(), b"keep");
    // The rest of the carried stage goes too: the launcher's own read-only
    // copy would refuse the new image's, and an old layer part would join it.
    assert!(!stage.join("index.json").exists());
    assert!(!stage.join("launch.py").exists());
    assert_eq!(
        std::fs::read(session.join("guest/workspace/notes.txt")).unwrap(),
        b"kept"
    );
    // A missing session is an error; one that staged nothing has nothing to forget.
    drop_carried_image(&dir.path().join("missing")).unwrap_err();
    std::fs::create_dir_all(dir.path().join("bare")).unwrap();
    drop_carried_image(&dir.path().join("bare")).unwrap();
}

/// A new owner (resume, cold boot, fork, `--from`) runs the recorded image:
/// the service knows it again, and its surface is granted afresh.
#[tokio::test]
async fn a_new_owner_restores_the_recorded_workload_without_its_old_exposure() {
    let dir = tempfile::tempdir().unwrap();
    let state = Arc::new(make_test_state_owned());
    let session = dir.path().join("session");
    write_record(&session, &record(Some("exposure-of-the-old-owner")));
    insert_fake_instance_with_session_dir(&state, "box", 1, session);

    restore(&state, "box");

    let live = state.containers.status("box").expect("restored");
    assert_eq!(live.state, ContainerState::Starting);
    assert_eq!(live.image, IMAGE);
    assert_eq!(live.digest.as_deref(), Some("sha256:aa"));
    assert_eq!(live.surface.unwrap().exposure_id, None);
    assert_eq!(
        state.containers.launch_record("box").unwrap().manifest.as_deref(),
        Some(MANIFEST),
        "the pin survives the next launch record"
    );

    restore(&state, "absent");
    assert!(state.containers.status("absent").is_none());
}

#[tokio::test]
async fn a_vm_without_a_record_is_not_given_a_workload() {
    let dir = tempfile::tempdir().unwrap();
    let state = Arc::new(make_test_state_owned());
    let session = dir.path().join("session");
    std::fs::create_dir_all(&session).unwrap();
    insert_fake_instance_with_session_dir(&state, "box", 1, session);
    restore(&state, "box");
    assert!(state.containers.status("box").is_none());
}

#[tokio::test]
async fn restored_workload_success_waits_for_this_boots_running_marker() {
    let dir = tempfile::tempdir().unwrap();
    let state = Arc::new(make_test_state_owned());
    let session = dir.path().join("session");
    write_record(&session, &record(None));
    staged(&session, &[capsem_core::container::STAGE_READY]);
    insert_fake_instance_with_session_dir(&state, "box", 1, session.clone());
    let restored = tokio::spawn({
        let state = Arc::clone(&state);
        async move { restore_ready(&state, "box").await }
    });
    while state.containers.status("box").is_none() {
        tokio::task::yield_now().await;
    }
    assert!(
        !restored.is_finished(),
        "a guest-ready VM still has a starting workload"
    );
    std::fs::write(stage_of(&session).join(capsem_core::container::STAGE_RUNNING), "1\n").unwrap();
    restored.await.unwrap().unwrap();
}

#[tokio::test]
async fn restored_workload_failure_is_not_lifecycle_success() {
    let dir = tempfile::tempdir().unwrap();
    let state = Arc::new(make_test_state_owned());
    let session = dir.path().join("session");
    write_record(&session, &record(None));
    staged(
        &session,
        &[
            capsem_core::container::STAGE_READY,
            capsem_core::container::STAGE_FAILED,
        ],
    );
    insert_fake_instance_with_session_dir(&state, "box", 1, session);
    let error = restore_ready(&state, "box").await.unwrap_err();
    assert_eq!(error.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(error.body.error.contains("Failed") && error.body.error.contains("did not start"));
}

#[tokio::test]
async fn a_bare_vm_does_not_wait_for_a_workload_marker() {
    let dir = tempfile::tempdir().unwrap();
    let state = Arc::new(make_test_state_owned());
    let session = dir.path().join("session");
    std::fs::create_dir_all(&session).unwrap();
    insert_fake_instance_with_session_dir(&state, "box", 1, session);
    tokio::time::timeout(std::time::Duration::from_secs(1), restore_ready(&state, "box"))
        .await
        .unwrap()
        .unwrap();
    assert!(state.containers.status("box").is_none());
}

/// capsem-init relaunches only a stage it finds `ready`; a first launch that
/// died before writing it leaves a stage nothing would start again. The new
/// owner relaunches exactly that case, and never one the boot already took.
#[tokio::test]
async fn only_a_staged_workload_that_never_got_ready_is_relaunched_by_the_owner() {
    let dir = tempfile::tempdir().unwrap();
    let state = Arc::new(make_test_state_owned());
    let session = dir.path().join("session");
    let stage = session.join("guest/workspace").join(capsem_core::container::STAGE);
    std::fs::create_dir_all(&stage).unwrap();
    insert_fake_instance_with_session_dir(&state, "box", 1, session);

    assert!(!needs_relaunch(&state, "box"), "nothing staged, nothing to launch");
    std::fs::write(stage.join("options.json"), b"{}").unwrap();
    assert!(needs_relaunch(&state, "box"), "staged but never ready");
    std::fs::write(stage.join(capsem_core::container::STAGE_READY), b"1\n").unwrap();
    assert!(
        !needs_relaunch(&state, "box"),
        "the boot relaunches a ready stage itself"
    );
}

fn stage_of(session_dir: &std::path::Path) -> std::path::PathBuf {
    session_dir
        .join(capsem_core::GUEST_SHARE_DIR)
        .join(capsem_core::session::WORKSPACE_DIR)
        .join(capsem_core::container::STAGE)
}

fn staged(session_dir: &std::path::Path, markers: &[&str]) {
    let stage = stage_of(session_dir);
    std::fs::create_dir_all(&stage).unwrap();
    for marker in markers {
        std::fs::write(stage.join(marker), "1\n").unwrap();
    }
}

/// A fork of a running session copies its `running` marker. Until the
/// fork's own launcher clears it, the service would report a workload that is
/// still unpacking as running and route exec to nothing.
#[test]
fn a_clone_starts_without_its_sources_run_markers() {
    use capsem_core::container::{STAGE_EXITED, STAGE_FAILED, STAGE_READY, STAGE_RUNNING};
    let dir = tempfile::tempdir().unwrap();
    let (source, clone) = (dir.path().join("source"), dir.path().join("clone"));
    write_record(&source, &record(None));
    staged(
        &clone,
        &[STAGE_READY, "options.json", STAGE_RUNNING, STAGE_FAILED, STAGE_EXITED],
    );

    carry_launch_record(&source, &clone).unwrap();

    let left: std::collections::BTreeSet<_> = std::fs::read_dir(stage_of(&clone))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(left, [STAGE_READY, "options.json"].map(String::from).into());
}

/// The stage is guest-written: a marker replaced by a link is removed as the
/// link, and what it pointed at is never touched.
#[test]
fn forgetting_a_run_never_follows_a_marker_link() {
    let dir = tempfile::tempdir().unwrap();
    let session = dir.path().join("session");
    staged(&session, &[]);
    let outside = dir.path().join("host-file");
    std::fs::write(&outside, "keep").unwrap();
    std::os::unix::fs::symlink(&outside, stage_of(&session).join(capsem_core::container::STAGE_RUNNING)).unwrap();

    forget_previous_run(&session).unwrap();

    assert!(std::fs::symlink_metadata(stage_of(&session).join(capsem_core::container::STAGE_RUNNING)).is_err());
    assert_eq!(std::fs::read_to_string(&outside).unwrap(), "keep");
    // A session that never staged an image has nothing to forget.
    forget_previous_run(&dir.path().join("bare")).unwrap();
}
