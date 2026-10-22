use super::*;
use std::os::unix::fs::{symlink, PermissionsExt};

fn stage(session: &std::path::Path) -> std::path::PathBuf {
    session
        .join(crate::GUEST_SHARE_DIR)
        .join(crate::session::WORKSPACE_DIR)
        .join(super::super::STAGE)
}

fn legacy_stage(session: &std::path::Path) -> std::path::PathBuf {
    let stage = stage(session);
    std::fs::create_dir_all(&stage).unwrap();
    std::fs::write(
        stage.join("options.json"),
        serde_json::to_vec(&serde_json::json!({
            "manifest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "env": {"KEEP": "yes"}
        }))
        .unwrap(),
    )
    .unwrap();
    std::fs::write(stage.join("launch.py"), b"legacy launcher").unwrap();
    std::fs::write(stage.join("runtime_console.py"), b"legacy console owner").unwrap();
    stage
}

#[test]
#[cfg(target_os = "linux")]
fn legacy_stage_is_upgraded_once_without_losing_options() {
    let root = tempfile::tempdir().unwrap();
    let session = root.path().join("session");
    let stage = legacy_stage(&session);

    assert!(refresh(&session).unwrap());
    let options: serde_json::Value =
        serde_json::from_slice(&std::fs::read(stage.join("options.json")).unwrap()).unwrap();
    assert_eq!(options["workspace_owner"], "image_user");
    assert_eq!(options["env"]["KEEP"], "yes");
    assert_eq!(std::fs::read(stage.join("launch.py")).unwrap(), super::super::LAUNCHER);
    for name in std::iter::once("options.json").chain(super::super::LAUNCHER_FILES.iter().map(|(name, _)| *name)) {
        let mode = std::fs::metadata(stage.join(name)).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "migration must not widen {name}");
    }
    assert!(!refresh(&session).unwrap(), "current stage must be idempotent");
}

#[test]
fn an_existing_owner_policy_does_not_keep_an_obsolete_launcher() {
    let root = tempfile::tempdir().unwrap();
    let session = root.path().join("session");
    let stage = legacy_stage(&session);
    let options = br#"{ "workspace_owner": "image_user", "env": {"KEEP":"yes"} }"#;
    std::fs::write(stage.join("options.json"), options).unwrap();
    std::fs::write(stage.parent().unwrap().join("user-data"), b"keep workspace").unwrap();

    assert!(refresh(&session).unwrap());
    assert_eq!(std::fs::read(stage.join("launch.py")).unwrap(), super::super::LAUNCHER);
    assert_eq!(std::fs::read(stage.join("options.json")).unwrap(), options);
    assert_eq!(
        std::fs::read(stage.parent().unwrap().join("user-data")).unwrap(),
        b"keep workspace"
    );
    let mode = std::fs::metadata(stage.join("launch.py")).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    assert!(!refresh(&session).unwrap());
}

#[test]
#[cfg(target_os = "macos")]
fn macos_launcher_refresh_preserves_the_image_ownership_contract() {
    let root = tempfile::tempdir().unwrap();
    let session = root.path().join("session");
    let stage = legacy_stage(&session);
    let options = std::fs::read(stage.join("options.json")).unwrap();

    assert!(refresh(&session).unwrap());
    assert_eq!(std::fs::read(stage.join("options.json")).unwrap(), options);
    assert_eq!(std::fs::read(stage.join("launch.py")).unwrap(), super::super::LAUNCHER);
    assert!(!refresh(&session).unwrap());
}

#[test]
fn missing_workspace_stage_or_options_is_not_a_migration() {
    let root = tempfile::tempdir().unwrap();
    let session = root.path().join("session");
    std::fs::create_dir_all(&session).unwrap();
    assert!(!refresh(&session).unwrap());
    std::fs::create_dir_all(stage(&session)).unwrap();
    assert!(!refresh(&session).unwrap());
}

#[test]
fn missing_launcher_is_repaired_for_a_valid_stage() {
    let root = tempfile::tempdir().unwrap();
    let session = root.path().join("session");
    let stage = legacy_stage(&session);
    std::fs::remove_file(stage.join("launch.py")).unwrap();
    assert!(refresh(&session).unwrap());
    assert_eq!(std::fs::read(stage.join("launch.py")).unwrap(), super::super::LAUNCHER);
}

#[test]
fn a_current_launcher_still_refreshes_its_missing_console_module() {
    let root = tempfile::tempdir().unwrap();
    let session = root.path().join("session");
    let stage = legacy_stage(&session);
    std::fs::write(stage.join("options.json"), br#"{"workspace_owner":"image_user"}"#).unwrap();
    std::fs::write(stage.join("launch.py"), super::super::LAUNCHER).unwrap();
    std::fs::remove_file(stage.join("runtime_console.py")).unwrap();
    assert!(refresh(&session).unwrap());
    assert_eq!(
        std::fs::read(stage.join("runtime_console.py")).unwrap(),
        include_bytes!("../../../../../guest/artifacts/container/runtime_console.py")
    );
    assert!(!refresh(&session).unwrap());
}

#[test]
fn control_file_links_are_refused_without_touching_their_targets() {
    for name in std::iter::once("options.json").chain(super::super::LAUNCHER_FILES.iter().map(|(name, _)| *name)) {
        let root = tempfile::tempdir().unwrap();
        let session = root.path().join("session");
        let stage = legacy_stage(&session);
        let outside = root.path().join("outside");
        std::fs::write(&outside, b"keep").unwrap();
        // The legacy fixture predates new modules. An old ordinary helper
        // can become a link just as an existing control file can.
        if !stage.join(name).exists() {
            std::fs::write(stage.join(name), b"old helper").unwrap();
        }
        std::fs::remove_file(stage.join(name)).unwrap();
        symlink(&outside, stage.join(name)).unwrap();

        assert!(refresh(&session).is_err(), "{name} link must be refused");
        assert_eq!(std::fs::read(&outside).unwrap(), b"keep");
    }
}

#[test]
fn oversized_options_are_refused() {
    let root = tempfile::tempdir().unwrap();
    let session = root.path().join("session");
    let stage = legacy_stage(&session);
    std::fs::File::options()
        .write(true)
        .open(stage.join("options.json"))
        .unwrap()
        .set_len(CONTROL_FILE_LIMIT + 1)
        .unwrap();
    let error = refresh(&session).unwrap_err().to_string();
    assert!(error.contains("control-file limit"), "{error}");
}
