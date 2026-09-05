use super::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

fn paths() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let uds = dir.path().join("vm.sock");
    (dir, uds.with_extension("ready"), uds.with_extension("launched"))
}

#[tokio::test]
async fn the_launch_sentinel_returns_long_before_the_ceiling() {
    let (_dir, ready, launched) = paths();
    let writer = {
        let launched = launched.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(60)).await;
            std::fs::File::create(launched).unwrap();
        })
    };
    let started = Instant::now();
    let outcome = wait_for_launch(&ready, &launched, || true, LAUNCH_CEILING).await;
    assert_eq!(outcome, LaunchWait::Launched);
    assert!(
        started.elapsed() < Duration::from_millis(300),
        "{:?}",
        started.elapsed()
    );
    writer.await.unwrap();
}

#[tokio::test]
async fn readiness_still_wins_when_the_guest_is_already_up() {
    let (_dir, ready, launched) = paths();
    std::fs::File::create(&ready).unwrap();
    std::fs::File::create(&launched).unwrap();
    assert_eq!(
        wait_for_launch(&ready, &launched, || true, LAUNCH_CEILING).await,
        LaunchWait::Ready
    );
}

#[tokio::test]
async fn an_instance_that_vanishes_before_any_sentinel_is_a_crash() {
    let (_dir, ready, launched) = paths();
    let alive = Arc::new(AtomicBool::new(true));
    let killer = {
        let alive = Arc::clone(&alive);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(40)).await;
            alive.store(false, Ordering::SeqCst);
        })
    };
    let started = Instant::now();
    let outcome = wait_for_launch(&ready, &launched, || alive.load(Ordering::SeqCst), LAUNCH_CEILING).await;
    assert_eq!(outcome, LaunchWait::Crashed);
    assert!(started.elapsed() < Duration::from_millis(300));
    killer.await.unwrap();
}

#[tokio::test]
async fn a_crash_after_launch_is_not_reported_by_create() {
    // The launch sentinel exists; the instance dying afterwards is exec's
    // to surface, not create's.
    let (_dir, ready, launched) = paths();
    std::fs::File::create(&launched).unwrap();
    assert_eq!(
        wait_for_launch(&ready, &launched, || false, LAUNCH_CEILING).await,
        LaunchWait::Launched
    );
}

#[tokio::test]
async fn nothing_within_the_ceiling_times_out_at_the_ceiling() {
    let (_dir, ready, launched) = paths();
    let ceiling = Duration::from_millis(120);
    let started = Instant::now();
    let outcome = wait_for_launch(&ready, &launched, || true, ceiling).await;
    let took = started.elapsed();
    assert_eq!(outcome, LaunchWait::TimedOut);
    assert!(
        took >= ceiling && took < ceiling + Duration::from_millis(300),
        "{took:?}"
    );
}

#[test]
fn the_ceiling_is_the_old_fixed_wait() {
    assert_eq!(LAUNCH_CEILING, Duration::from_millis(500));
}
