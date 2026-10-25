use super::*;
use std::sync::Arc;
use tokio::sync::Semaphore;

#[tokio::test(start_paused = true)]
async fn concurrent_demo_requests_wait_for_body_capacity_instead_of_returning_502() {
    let budget = Arc::new(Semaphore::new(2));
    let first = admit(&budget).await.unwrap();
    let _second = admit(&budget).await.unwrap();
    let mut queued = tokio::spawn({
        let budget = Arc::clone(&budget);
        async move { admit(&budget).await }
    });
    tokio::task::yield_now().await;
    assert!(!queued.is_finished(), "a normal concurrency burst must wait, not fail");
    assert_eq!(budget.available_permits(), 0);
    drop(first);
    let third = (&mut queued).await.unwrap().unwrap();
    drop(queued);
    assert_eq!(budget.available_permits(), 0);
    drop(third);
    assert_eq!(budget.available_permits(), 1);
}

#[tokio::test(start_paused = true)]
async fn stalled_body_admission_is_bounded_and_does_not_bypass_inspection() {
    let budget = Arc::new(Semaphore::new(0));
    let mut queued = tokio::spawn({
        let budget = Arc::clone(&budget);
        async move { admit(&budget).await }
    });
    tokio::task::yield_now().await;
    assert!(!queued.is_finished());
    tokio::time::advance(BODY_DEADLINE).await;
    let error = (&mut queued).await.unwrap().unwrap_err();
    drop(queued);
    assert!(error.to_string().contains("timed out"));
}
