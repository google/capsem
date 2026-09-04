use super::*;
use std::sync::atomic::AtomicUsize;
use std::time::Duration;

/// A fetch that records when it started and takes `delay` to finish.
fn slow_fetch(
    counter: Arc<AtomicUsize>,
    delay: Duration,
) -> impl FnOnce() -> std::pin::Pin<Box<dyn Future<Output = usize> + Send>> {
    move || {
        let started_as = counter.fetch_add(1, Ordering::SeqCst) + 1;
        Box::pin(async move {
            tokio::time::sleep(delay).await;
            started_as
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn eight_concurrent_polls_perform_at_most_two_fetches() {
    let gate = Arc::new(RefreshGate::<usize>::new());
    let fetches = Arc::new(AtomicUsize::new(0));
    let mut polls = Vec::new();
    for _ in 0..8 {
        let gate = Arc::clone(&gate);
        let fetches = Arc::clone(&fetches);
        polls.push(tokio::spawn(async move {
            gate.read(slow_fetch(fetches, Duration::from_millis(50))).await
        }));
    }
    let mut led = 0;
    for poll in polls {
        let read = poll.await.unwrap();
        led += usize::from(read.led);
        assert!(*read.value >= 1);
    }
    assert!(
        fetches.load(Ordering::SeqCst) <= 2,
        "{} fetches for eight polls",
        fetches.load(Ordering::SeqCst)
    );
    assert_eq!(led as u64, gate.fetches_started(), "exactly one poll leads each fetch");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_poll_arriving_during_a_fetch_is_answered_by_a_fetch_that_began_after_it() {
    let gate = Arc::new(RefreshGate::<usize>::new());
    let fetches = Arc::new(AtomicUsize::new(0));
    let first = {
        let gate = Arc::clone(&gate);
        let fetches = Arc::clone(&fetches);
        tokio::spawn(async move { gate.read(slow_fetch(fetches, Duration::from_millis(150))).await })
    };
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(fetches.load(Ordering::SeqCst), 1, "the first fetch is in flight");
    // This poll arrives while fetch #1 runs. Fetch #1 began before it, so
    // its result must not be served here.
    let late = gate
        .read(slow_fetch(Arc::clone(&fetches), Duration::from_millis(10)))
        .await;
    assert_eq!(
        *late.value, 2,
        "answered by the second fetch, which began after the poll arrived"
    );
    assert_eq!(*first.await.unwrap().value, 1);
}

#[tokio::test]
async fn sequential_polls_each_fetch_because_nothing_is_cached() {
    let gate = RefreshGate::<usize>::new();
    let fetches = Arc::new(AtomicUsize::new(0));
    for expected in 1..=3 {
        let read = gate.read(slow_fetch(Arc::clone(&fetches), Duration::ZERO)).await;
        assert_eq!(*read.value, expected);
        assert!(read.led);
    }
    assert_eq!(gate.fetches_started(), 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn followers_of_a_fetch_all_receive_the_same_read_and_do_not_lead() {
    let gate = Arc::new(RefreshGate::<usize>::new());
    let fetches = Arc::new(AtomicUsize::new(0));
    // Fetch #1 in flight; the next wave arrives during it and must share #2.
    let leader = {
        let gate = Arc::clone(&gate);
        let fetches = Arc::clone(&fetches);
        tokio::spawn(async move { gate.read(slow_fetch(fetches, Duration::from_millis(100))).await })
    };
    tokio::time::sleep(Duration::from_millis(20)).await;
    let mut wave = Vec::new();
    for _ in 0..6 {
        let gate = Arc::clone(&gate);
        let fetches = Arc::clone(&fetches);
        wave.push(tokio::spawn(async move {
            gate.read(slow_fetch(fetches, Duration::from_millis(30))).await
        }));
    }
    leader.await.unwrap();
    let reads: Vec<Read<usize>> = futures::future::join_all(wave)
        .await
        .into_iter()
        .map(|r| r.unwrap())
        .collect();
    assert!(reads.iter().all(|r| *r.value == 2), "the whole wave shares fetch #2");
    assert_eq!(reads.iter().filter(|r| r.led).count(), 1);
    assert_eq!(fetches.load(Ordering::SeqCst), 2);
}
