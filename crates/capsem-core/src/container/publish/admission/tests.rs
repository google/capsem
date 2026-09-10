use super::*;
use std::time::Duration;
use tokio::time::{advance, timeout, Instant};

#[tokio::test(start_paused = true)]
async fn burst_is_bounded_and_refills_at_32_per_second() {
    let rate = SetupRate::default();
    for _ in 0..16 {
        timeout(Duration::ZERO, rate.acquire()).await.unwrap();
    }
    assert!(
        timeout(Duration::ZERO, rate.acquire()).await.is_err(),
        "setup burst was unbounded"
    );
    let start = Instant::now();
    rate.acquire().await;
    // Tokio timers round to millisecond ticks; pacing must never run early.
    assert!((Duration::from_micros(31_250)..=Duration::from_micros(32_250)).contains(&start.elapsed()));
    let start = Instant::now();
    for _ in 0..32 {
        rate.acquire().await;
    }
    assert!((Duration::from_millis(999)..=Duration::from_millis(1001)).contains(&start.elapsed()));
    advance(Duration::from_secs(600)).await;
    for _ in 0..16 {
        timeout(Duration::ZERO, rate.acquire()).await.unwrap();
    }
    assert!(
        timeout(Duration::ZERO, rate.acquire()).await.is_err(),
        "idle time enlarged the burst"
    );
}

#[tokio::test(start_paused = true)]
async fn cancelled_waiter_preserves_credit_and_other_class_progress() {
    let private = SetupRate::default();
    let ingress = SetupRate::default();
    for _ in 0..16 {
        private.acquire().await;
    }
    let start = Instant::now();
    assert!(timeout(Duration::from_millis(10), private.acquire()).await.is_err());
    timeout(Duration::ZERO, ingress.acquire()).await.unwrap();
    private.acquire().await;
    assert!((Duration::from_micros(31_250)..=Duration::from_micros(32_250)).contains(&start.elapsed()));
}
