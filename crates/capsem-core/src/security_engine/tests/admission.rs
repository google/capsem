//! A fail-closed action waits a bounded time for its audit record.
//!
//! The async admission path waits as long as the writer queue stays full, so a
//! stalled writer hung a DNS answer or a guest file write indefinitely instead
//! of refusing it (google/capsem#225, owned by #229).
use super::*;

#[tokio::test(start_paused = true)]
async fn an_admission_that_outlives_its_deadline_is_refused() {
    let started = tokio::time::Instant::now();
    let stalled = admit_within(
        SECURITY_ADMISSION_DEADLINE,
        std::future::pending::<Option<SecurityEventId>>(),
    )
    .await;
    assert!(stalled.is_none(), "a record never accepted must read as refused");
    assert_eq!(started.elapsed(), SECURITY_ADMISSION_DEADLINE);
}

#[tokio::test(start_paused = true)]
async fn an_admission_within_its_deadline_keeps_its_answer() {
    let accepted = admit_within(SECURITY_ADMISSION_DEADLINE, async {
        tokio::time::sleep(SECURITY_ADMISSION_DEADLINE / 2).await;
        Some(7)
    })
    .await;
    assert_eq!(accepted, Some(7));
    let refused = admit_within(SECURITY_ADMISSION_DEADLINE, async { None::<u8> }).await;
    assert_eq!(refused, None, "an explicit refusal stays a refusal");
}
