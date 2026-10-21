use super::*;

fn blob(digest: &str, completed_bytes: u64, total_bytes: u64, verified: bool) -> BlobProgress {
    BlobProgress {
        digest: digest.to_owned(),
        completed_bytes,
        total_bytes,
        verified,
    }
}

#[test]
fn total_stays_unknown_until_the_final_transfer_is_declared() {
    let downloads = Arc::new(Downloads::default());
    let pull = downloads.begin("one");
    assert_eq!(downloads.snapshot("one").unwrap().phase, ImageDownloadPhase::Preparing);
    let image = pull.observer(false);
    image(blob("config", 10, 10, true));
    image(blob("layer", 50, 100, false));
    let progress = downloads.snapshot("one").unwrap();
    assert_eq!(progress.completed_bytes, 60);
    assert_eq!(progress.total_bytes, None);
    image(blob("layer", 100, 100, true));
    let root = pull.observer(true);
    root(blob("root", 30, 50, false));
    let progress = downloads.snapshot("one").unwrap();
    assert_eq!(progress.completed_bytes, 140);
    assert_eq!(progress.total_bytes, Some(160));
    assert_eq!(progress.phase, ImageDownloadPhase::Downloading);
    root(blob("root", 50, 50, false));
    assert_eq!(downloads.snapshot("one").unwrap().phase, ImageDownloadPhase::Verifying);
    root(blob("root", 50, 50, true));
    drop(pull);
    assert!(downloads.snapshot("one").is_none());
    root(blob("root", 50, 50, true));
    assert!(
        downloads.snapshot("one").is_none(),
        "a late callback cannot resurrect an ended request"
    );
}

#[test]
fn concurrent_requests_and_repeated_digests_keep_independent_counts() {
    let downloads = Arc::new(Downloads::default());
    let older = downloads.begin("one");
    older.observer(false)(blob("layer", 20, 100, false));
    let newer = downloads.begin("one");
    newer.observer(false)(blob("layer", 60, 100, false));
    let other = downloads.begin("two");
    other.observer(false)(blob("layer", 80, 100, false));
    assert_eq!(downloads.snapshot("one").unwrap().completed_bytes, 60);
    assert_eq!(downloads.snapshot("two").unwrap().completed_bytes, 80);
    drop(newer);
    assert_eq!(downloads.snapshot("one").unwrap().completed_bytes, 20);
    let observer = older.observer(false);
    observer(blob("layer", 0, 100, false));
    observer(blob("layer", 19, 100, false));
    observer(blob("layer", 70, 100, false));
    assert_eq!(downloads.snapshot("one").unwrap().completed_bytes, 70);
    observer(blob("layer", 90, 99, false));
    observer(blob("bad", 200, 100, false));
    assert_eq!(downloads.snapshot("one").unwrap().completed_bytes, 70);
    drop(older);
    drop(other);
    assert!(downloads.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn cancellation_and_timeout_drop_the_only_advertised_state() {
    let downloads = Arc::new(Downloads::default());
    let pull = downloads.begin("cancelled");
    let task = tokio::spawn(async move {
        let _pull = pull;
        std::future::pending::<()>().await;
    });
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(downloads.snapshot("cancelled").is_none());
    let pull = downloads.begin("timeout");
    let result = tokio::time::timeout(std::time::Duration::from_millis(1), async move {
        let _pull = pull;
        std::future::pending::<()>().await;
    })
    .await;
    assert!(result.is_err());
    assert!(downloads.snapshot("timeout").is_none());
    assert!(downloads.0.lock().unwrap().is_empty());
}
