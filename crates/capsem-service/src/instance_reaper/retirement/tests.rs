use super::*;
use capsem_foundation::unix::process::{Signal, SignalOutcome};
use std::time::Duration;

fn sleeper() -> tokio::process::Child {
    tokio::process::Command::new("sleep")
        .arg("30")
        .env_clear()
        .kill_on_drop(true)
        .spawn()
        .unwrap()
}

#[tokio::test]
async fn owned_child_exit_precedes_full_retirement_and_stale_signals_do_not_retarget() {
    let owner = std::sync::Arc::new(Retirements::default());
    let old_generation = capsem_crypto::uuid_v4();
    let mut registration = owner.register("vm", old_generation).unwrap();
    let mut child = sleeper();
    assert!(owner.is_live("vm", old_generation).unwrap());
    let (released, release) = tokio::sync::oneshot::channel();
    let reaper = tokio::spawn(async move {
        let result = registration.wait_child(&mut child).await.unwrap();
        assert!(!result.success());
        release.await.unwrap();
        registration.complete();
    });
    assert!(owner
        .signal("vm", capsem_crypto::uuid_v4(), Signal::Kill)
        .await
        .is_err());
    assert_eq!(
        owner.signal("vm", old_generation, Signal::Kill).await.unwrap(),
        SignalOutcome::Delivered
    );
    assert!(owner.wait_exit("vm", old_generation).await.unwrap());
    assert!(!owner.is_live("vm", old_generation).unwrap());
    let full = {
        let owner = owner.clone();
        tokio::spawn(async move { owner.wait("vm", old_generation).await })
    };
    tokio::task::yield_now().await;
    assert!(!full.is_finished(), "child exit is not session retirement");
    let new_generation = capsem_crypto::uuid_v4();
    let mut new_registration = owner.register("vm", new_generation).unwrap();
    let mut replacement = sleeper();
    assert_eq!(
        owner.signal("vm", old_generation, Signal::Kill).await.unwrap(),
        SignalOutcome::Gone
    );
    assert!(
        replacement.try_wait().unwrap().is_none(),
        "stale signal killed replacement"
    );
    let replacement_owner = tokio::spawn(async move {
        new_registration.wait_child(&mut replacement).await.unwrap();
        new_registration.complete();
    });
    owner.signal("vm", new_generation, Signal::Kill).await.unwrap();
    replacement_owner.await.unwrap();
    released.send(()).unwrap();
    reaper.await.unwrap();
    assert!(full.await.unwrap().unwrap());
}

#[tokio::test]
async fn interrupted_reaper_is_not_exit_or_signal_authority() {
    let owner = Retirements::default();
    let generation = capsem_crypto::uuid_v4();
    let registration = owner.register("vm", generation).unwrap();
    drop(registration);
    assert!(owner.wait_exit("vm", generation).await.is_err());
    assert!(owner.signal("vm", generation, Signal::Kill).await.is_err());
    assert!(owner.wait("vm", generation).await.is_err());
    assert!(
        owner.wait_exit("missing", generation).await.is_err(),
        "absence is not exit"
    );
}

#[tokio::test]
async fn owned_signal_outlives_its_waiter_and_requires_the_original_generation() {
    let owner = std::sync::Arc::new(Retirements::default());
    let generation = capsem_crypto::uuid_v4();
    let mut registration = owner.register("vm", generation).unwrap();
    let mut child = sleeper();
    let request = {
        let owner = owner.clone();
        tokio::spawn(async move { owner.signal("vm", generation, Signal::Kill).await })
    };
    tokio::time::timeout(Duration::from_secs(1), async {
        while registration.signals.is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    request.abort();
    assert!(request.await.unwrap_err().is_cancelled());
    let status = tokio::time::timeout(Duration::from_secs(1), registration.wait_child(&mut child))
        .await
        .unwrap()
        .unwrap();
    assert!(!status.success());
    registration.complete();
    assert!(owner.wait_exit("vm", generation).await.unwrap());
}

#[tokio::test]
async fn observed_native_spawn_failure_has_positive_no_child_retirement() {
    let owner = Retirements::default();
    let generation = capsem_crypto::uuid_v4();
    let directory = tempfile::tempdir().unwrap();
    let mut command = tokio::process::Command::new(directory.path().join("missing-binary"));
    let mut unrelated = sleeper();
    assert!(owner.spawn("failed", generation, &mut command).is_err());
    assert!(owner.wait_exit("failed", generation).await.unwrap());
    assert!(owner.wait("failed", generation).await.unwrap());
    assert!(!owner.is_live("failed", generation).unwrap());
    assert_eq!(
        owner.signal("failed", generation, Signal::Kill).await.unwrap(),
        SignalOutcome::Gone
    );
    assert!(unrelated.try_wait().unwrap().is_none());
    unrelated.kill().await.unwrap();
}

#[tokio::test]
async fn post_spawn_failure_reaps_owned_child_before_publishing_retirement() {
    let owner = Retirements::default();
    let generation = capsem_crypto::uuid_v4();
    let mut command = tokio::process::Command::new("sleep");
    command.arg("30").env_clear().kill_on_drop(true);
    let (child, registration) = owner.spawn("failed", generation, &mut command).unwrap();
    let mut unrelated = sleeper();
    let reaper = crate::instance_reaper::kill_and_reap(child, registration);
    tokio::time::timeout(Duration::from_secs(1), reaper)
        .await
        .unwrap()
        .unwrap();
    assert!(owner.wait_exit("failed", generation).await.unwrap());
    assert!(owner.wait("failed", generation).await.unwrap());
    assert!(!owner.is_live("failed", generation).unwrap());
    assert!(unrelated.try_wait().unwrap().is_none());
    unrelated.kill().await.unwrap();
}
