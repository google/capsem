use super::*;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Barrier,
};

#[test]
fn concurrent_launches_remain_independent_and_refusal_preserves_admission() {
    let lifecycle = VmLifecycle::default();
    let first = lifecycle.admit().unwrap();
    let second = lifecycle.admit().unwrap();
    assert_eq!(
        lifecycle.begin_restart(|| panic!("must refuse before querying instances")),
        Err(RestartDenied::LaunchInProgress)
    );
    drop(first);
    assert_eq!(lifecycle.begin_restart(|| false), Err(RestartDenied::LaunchInProgress));
    drop(second);
    assert!(lifecycle.admit().is_ok());
    assert_eq!(lifecycle.begin_restart(|| true), Err(RestartDenied::ActiveVms));
    assert!(lifecycle.admit().is_ok());
    lifecycle.begin_restart(|| false).unwrap();
    assert!(matches!(lifecycle.admit(), Err(RestartDenied::AlreadyRequested)));
    assert_eq!(
        lifecycle.begin_restart(|| panic!("do not recheck after acceptance")),
        Err(RestartDenied::AlreadyRequested)
    );
}

#[test]
fn registration_remains_excluded_until_the_worker_publishes_its_instance() {
    let lifecycle = VmLifecycle::default();
    let active = AtomicBool::new(false);
    let launched = Barrier::new(2);
    let register = Barrier::new(2);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let _permit = lifecycle.admit().unwrap();
            launched.wait();
            register.wait();
            active.store(true, Ordering::SeqCst);
        });
        launched.wait();
        assert_eq!(
            lifecycle.begin_restart(|| active.load(Ordering::SeqCst)),
            Err(RestartDenied::LaunchInProgress)
        );
        register.wait();
    });
    assert_eq!(
        lifecycle.begin_restart(|| active.load(Ordering::SeqCst)),
        Err(RestartDenied::ActiveVms)
    );
}

#[tokio::test]
async fn cancelled_request_cannot_release_a_blocking_workers_launch_permit() {
    let lifecycle = Arc::new(VmLifecycle::default());
    let active = Arc::new(AtomicBool::new(false));
    let (started, observed) = tokio::sync::oneshot::channel();
    let (release, released) = std::sync::mpsc::channel();
    let (finished, complete) = tokio::sync::oneshot::channel();
    let request = tokio::spawn({
        let lifecycle = lifecycle.clone();
        let active = active.clone();
        async move {
            tokio::task::spawn_blocking(move || {
                let permit = lifecycle.admit().unwrap();
                started.send(()).unwrap();
                released.recv().unwrap();
                active.store(true, Ordering::SeqCst);
                drop(permit);
                finished.send(()).unwrap();
            })
            .await
            .unwrap();
        }
    });
    observed.await.unwrap();
    request.abort();
    assert!(request.await.unwrap_err().is_cancelled());
    let denied = lifecycle.begin_restart(|| active.load(Ordering::SeqCst));
    release.send(()).unwrap();
    complete.await.unwrap();
    assert_eq!(denied, Err(RestartDenied::LaunchInProgress));
    assert_eq!(
        lifecycle.begin_restart(|| active.load(Ordering::SeqCst)),
        Err(RestartDenied::ActiveVms)
    );
}

#[test]
fn typed_denials_explain_the_required_action() {
    for (reason, message) in [
        (RestartDenied::LaunchInProgress, "a VM launch is in progress"),
        (
            RestartDenied::ActiveVms,
            "stop all active VMs before restarting the service",
        ),
        (
            RestartDenied::AlreadyRequested,
            "service restart has already been requested",
        ),
    ] {
        assert_eq!(reason.to_string(), message);
    }
}

#[tokio::test]
async fn shutdown_seals_admission_and_joins_previously_admitted_workers() {
    let lifecycle = Arc::new(VmLifecycle::default());
    let first = lifecycle.admit().unwrap();
    let second = lifecycle.admit().unwrap();
    lifecycle.begin_shutdown();
    assert!(matches!(lifecycle.admit(), Err(RestartDenied::AlreadyRequested)));
    let drained = {
        let lifecycle = lifecycle.clone();
        tokio::spawn(async move { lifecycle.wait_for_launches().await })
    };
    tokio::task::yield_now().await;
    assert!(!drained.is_finished());
    drop(first);
    tokio::task::yield_now().await;
    assert!(!drained.is_finished(), "one launch is still owned");
    drop(second);
    tokio::time::timeout(std::time::Duration::from_secs(1), drained)
        .await
        .unwrap()
        .unwrap();
    lifecycle.begin_shutdown();
    lifecycle.wait_for_launches().await;
    assert!(matches!(lifecycle.admit(), Err(RestartDenied::AlreadyRequested)));
}

#[tokio::test]
async fn shutdown_drain_observes_launch_completion_before_it_starts_waiting() {
    let lifecycle = VmLifecycle::default();
    let worker = lifecycle.admit().unwrap();
    lifecycle.begin_shutdown();
    drop(worker);
    tokio::time::timeout(std::time::Duration::from_secs(1), lifecycle.wait_for_launches())
        .await
        .unwrap();
}
