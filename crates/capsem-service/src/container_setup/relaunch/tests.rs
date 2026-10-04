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
        state.containers.manifest("box").as_deref(),
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
