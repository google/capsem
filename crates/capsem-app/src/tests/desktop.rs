//! Real process and session-bus acceptance of the plugin used by the shell.
use super::*;
use std::process::Command;
use tauri::test::{mock_builder, mock_context, noop_assets};

const STAGE: &str = "CAPSEM_TEST_DESKTOP_STAGE";
const NAME: &str = "CAPSEM_TEST_DESKTOP_NAME";

#[test]
fn second_launch_activates_the_primary_on_a_private_bus() {
    let status = Command::new("dbus-run-session")
        .arg("--")
        .arg(std::env::current_exe().unwrap())
        .args(["--exact", "tests::desktop::activation_in_private_bus", "--nocapture"])
        .env(STAGE, "primary")
        .status()
        .unwrap();
    assert!(status.success());
}

#[test]
fn activation_in_private_bus() {
    if std::env::var(STAGE).as_deref() != Ok("primary") {
        return;
    }
    let name = format!("com.capsem.desktop_test.p{}", std::process::id());
    let (sender, receiver) = std::sync::mpsc::channel();
    let app = mock_builder()
        .plugin(
            tauri_plugin_single_instance::Builder::new()
                .dbus_id(&name)
                .callback(move |_, args, cwd| {
                    sender.send((args, cwd)).unwrap();
                })
                .build(),
        )
        .build(mock_context(noop_assets()))
        .unwrap();
    let directory = tempfile::Builder::new()
        .prefix("activation with spaces ")
        .tempdir()
        .unwrap();
    let executable = std::env::current_exe().unwrap();
    let child_args = ["--exact", "tests::desktop::secondary_launch", "--nocapture"];
    let status = Command::new(&executable)
        .args(child_args)
        .env(STAGE, "secondary")
        .env(NAME, &name)
        .current_dir(directory.path())
        .status()
        .unwrap();
    assert!(status.success());
    let (args, cwd) = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
    let expected = std::iter::once(executable.to_string_lossy().into_owned())
        .chain(child_args.map(String::from))
        .collect::<Vec<_>>();
    assert_eq!(args, expected);
    assert_eq!(cwd, directory.path().to_string_lossy());
    assert!(receiver.try_recv().is_err(), "one launch must activate exactly once");
    tauri_plugin_single_instance::destroy(&app);
    let replacement = mock_builder()
        .plugin(tauri_plugin_single_instance::Builder::new().dbus_id(name).build())
        .build(mock_context(noop_assets()))
        .unwrap();
    tauri_plugin_single_instance::destroy(&replacement);
}

#[test]
fn secondary_launch() {
    if std::env::var(STAGE).as_deref() != Ok("secondary") {
        return;
    }
    let _app = mock_builder()
        .plugin(
            tauri_plugin_single_instance::Builder::new()
                .dbus_id(std::env::var(NAME).unwrap())
                .build(),
        )
        .build(mock_context(noop_assets()))
        .unwrap();
    panic!("secondary must exit only after the primary acknowledged activation");
}

#[test]
fn native_url_launcher_receives_only_allowed_urls_as_single_arguments() {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir().unwrap();
    let launcher = directory.path().join("xdg-open");
    fs::write(
        &launcher,
        "#!/bin/sh\nprintf '%s\\n' \"$#\" \"$1\" >> \"$CAPSEM_TEST_OPEN_LOG\"\n",
    )
    .unwrap();
    fs::set_permissions(&launcher, fs::Permissions::from_mode(0o700)).unwrap();
    let log = directory.path().join("open.log");
    let status = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "tests::desktop::native_url_launcher_child", "--nocapture"])
        .env(STAGE, "opener")
        .env("PATH", directory.path())
        .env("CAPSEM_TEST_OPEN_LOG", &log)
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(
        fs::read_to_string(log).unwrap().lines().collect::<Vec<_>>(),
        [
            "1",
            "https://example.test/a?q=space and 'quotes'&next=$(literal)",
            "1",
            "mailto:recipient@example.test?subject=hello world",
        ]
    );
}

#[test]
fn native_url_launcher_child() {
    if std::env::var(STAGE).as_deref() != Ok("opener") {
        return;
    }
    let app = mock_builder()
        .manage(browser::Callbacks::default())
        .build(mock_context(noop_assets()))
        .unwrap();
    let log = PathBuf::from(std::env::var("CAPSEM_TEST_OPEN_LOG").unwrap());
    for denied in [
        "file:///tmp/private",
        "javascript:alert(1)",
        "data:text/html,secret",
        "--load-modules=/tmp/plugin",
    ] {
        assert!(tauri::async_runtime::block_on(open_url(denied.into(), None, app.state())).is_err());
    }
    assert!(!log.exists(), "denied URLs must never invoke an OS launcher");
    for (index, url) in [
        "https://example.test/a?q=space and 'quotes'&next=$(literal)",
        "mailto:recipient@example.test?subject=hello world",
    ]
    .into_iter()
    .enumerate()
    {
        tauri::async_runtime::block_on(open_url(url.into(), None, app.state())).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if fs::read_to_string(&log).is_ok_and(|text| text.lines().count() == (index + 1) * 2) {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "native launcher did not receive URL"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }
}
