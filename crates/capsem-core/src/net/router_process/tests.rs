use super::*;
use capsem_foundation::unix::process::{probe, ProcessId, ProcessState};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

/// A stand-in router: records its pid, writes `answer` back on the startup
/// socket (its stdin), then does `then`.
fn stand_in(dir: &std::path::Path, answer: &[u8], then: &str) -> PathBuf {
    let escaped = answer.iter().fold(String::new(), |mut escaped, byte| {
        use std::fmt::Write;
        write!(escaped, "\\{byte:03o}").unwrap();
        escaped
    });
    let script = dir.join("capsem-router");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\necho $$ > '{}'\nprintf '{escaped}' >&0\n{then}\n",
            dir.join("pid").display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    script
}

async fn encoded(event: Event) -> Vec<u8> {
    let mut bytes = Vec::new();
    event.write(&mut bytes).await.unwrap();
    bytes
}

fn stand_in_pid(dir: &std::path::Path) -> ProcessId {
    let pid: u32 = std::fs::read_to_string(dir.join("pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    ProcessId::try_from(pid).unwrap()
}

#[tokio::test]
async fn a_router_that_cannot_confine_itself_is_refused_and_reaped() {
    let dir = tempfile::tempdir().unwrap();
    let binary = stand_in(dir.path(), &encoded(Event::ConfinementFailed).await, "exec sleep 30");
    let error = spawn_from(&binary, &[])
        .await
        .err()
        .expect("an unconfined router is refused");
    assert!(
        format!("{error:#}").contains("could not install its sandbox"),
        "{error:#}"
    );
    assert_eq!(probe(stand_in_pid(dir.path())).unwrap(), ProcessState::Gone);
}

#[tokio::test]
async fn a_router_that_answers_anything_but_ready_is_refused_and_reaped() {
    let dir = tempfile::tempdir().unwrap();
    let binary = stand_in(dir.path(), &encoded(Event::Refused(7)).await, "exec sleep 30");
    let error = spawn_from(&binary, &[])
        .await
        .err()
        .expect("an unconfirmed router is refused");
    assert!(
        format!("{error:#}").contains("did not confirm confinement"),
        "{error:#}"
    );
    assert_eq!(probe(stand_in_pid(dir.path())).unwrap(), ProcessState::Gone);
}

#[tokio::test]
async fn a_router_that_exits_before_confirming_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let binary = stand_in(dir.path(), &[], "exit 0");
    let error = spawn_from(&binary, &[])
        .await
        .err()
        .expect("a vanished router is refused");
    assert!(
        format!("{error:#}").contains("closed its startup channel before confirming confinement"),
        "{error:#}"
    );
    assert_eq!(probe(stand_in_pid(dir.path())).unwrap(), ProcessState::Gone);
}

#[tokio::test]
async fn a_router_that_never_confirms_is_refused_at_the_deadline_and_reaped() {
    let dir = tempfile::tempdir().unwrap();
    let binary = stand_in(dir.path(), &[], "exec sleep 30");
    let started = std::time::Instant::now();
    let error = spawn_from(&binary, &[])
        .await
        .err()
        .expect("a silent router is refused");
    assert!(format!("{error:#}").contains("timed out"), "{error:#}");
    assert!(started.elapsed() < Duration::from_secs(8));
    assert_eq!(probe(stand_in_pid(dir.path())).unwrap(), ProcessState::Gone);
}
