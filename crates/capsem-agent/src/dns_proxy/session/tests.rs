use super::*;
use crate::wire::fixtures::{answer_to, query};
use std::io::{Read, Write};
use std::os::fd::IntoRawFd;
use std::os::unix::net::UnixStream;
use std::sync::atomic::AtomicBool;

fn idle_session(capacity: usize) -> (DnsSession, mpsc::Receiver<(u32, Vec<u8>)>) {
    let (frames, receiver) = mpsc::channel(capacity);
    (
        DnsSession {
            index: 0,
            frames,
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicU32::new(1),
        },
        receiver,
    )
}

#[tokio::test]
async fn cancellation_releases_the_pending_slot() {
    let (session, _receiver) = idle_session(1);
    let mut query = Box::pin(session.forward_query(query(1, "cancel.example"), "udp"));
    std::future::poll_fn(|cx| {
        assert!(std::future::Future::poll(query.as_mut(), cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    assert_eq!(session.in_flight(), 1);
    drop(query);
    assert_eq!(session.in_flight(), 0, "cancelled requests must free their slot");
}

#[tokio::test]
async fn the_deadline_includes_waiting_for_writer_capacity() {
    let (session, _receiver) = idle_session(1);
    session.frames.try_send((0, Vec::new())).unwrap();
    let result = tokio::time::timeout(
        Duration::from_millis(200),
        session.forward_query_within(query(1, "queued.example"), "udp", Duration::from_millis(20)),
    )
    .await
    .expect("the query deadline must also bound a full writer queue");
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::TimedOut);
    assert_eq!(session.in_flight(), 0);
}

#[tokio::test]
async fn expired_frames_are_not_replayed_when_a_connection_recovers() {
    let (session, mut frames) = idle_session(4);
    let session = Arc::new(session);
    assert!(session
        .forward_query_within(query(1, "expired.example"), "udp", Duration::from_millis(10))
        .await
        .is_err());
    let (mut host, guest) = UnixStream::pair().unwrap();
    host.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let pump = {
        let session = Arc::clone(&session);
        tokio::spawn(async move {
            session
                .pump(AsyncVsock::new(guest.into_raw_fd()).unwrap(), &mut frames)
                .await
        })
    };
    let host = tokio::task::spawn_blocking(move || {
        let request = read_frame(&mut host).unwrap();
        assert_eq!(
            request.raw,
            query(2, "current.example"),
            "expired traffic must never reach the recovered host"
        );
        write_response(&mut host, &answered(&request));
        host
    });
    let answer = session.forward_query(query(2, "current.example"), "udp").await.unwrap();
    assert_eq!(answer.raw, answer_to(&query(2, "current.example")));
    drop(host.await.unwrap());
    pump.await.unwrap();
}

#[tokio::test]
async fn dropping_the_forwarder_closes_its_session_connection() {
    let (mut host, guest) = UnixStream::pair().unwrap();
    host.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let guest = Arc::new(Mutex::new(Some(guest)));
    let connect: Connect = Arc::new(move || {
        guest
            .lock()
            .unwrap()
            .take()
            .map(IntoRawFd::into_raw_fd)
            .ok_or_else(|| io::Error::other("one connection only"))
    });
    let forwarder = DnsForwarder::new(1, connect);
    let host = tokio::task::spawn_blocking(move || {
        let request = read_frame(&mut host).unwrap();
        write_response(&mut host, &answered(&request));
        host
    });
    forwarder.forward_query(query(1, "done.example"), "udp").await.unwrap();
    let mut host = host.await.unwrap();
    drop(forwarder);
    let eof = tokio::task::spawn_blocking(move || host.read(&mut [0u8]))
        .await
        .unwrap();
    assert_eq!(eof.unwrap(), 0, "dropping the owner must close the connection");
}

/// One frame read from the host end of the pair.
fn read_frame(host: &mut UnixStream) -> Option<DnsRequest> {
    let mut len_buf = [0u8; 4];
    host.read_exact(&mut len_buf).ok()?;
    let mut payload = vec![0u8; u32::from_be_bytes(len_buf) as usize];
    host.read_exact(&mut payload).ok()?;
    Some(capsem_proto::decode_dns_request(&payload).unwrap())
}

fn write_response(host: &mut UnixStream, response: &DnsResponse) {
    let frame = capsem_proto::encode_dns_response(response).unwrap();
    host.write_all(&frame).unwrap();
}

fn answered(request: &DnsRequest) -> DnsResponse {
    DnsResponse {
        id: request.id,
        raw: answer_to(&request.raw),
        decision: "allowed".into(),
        rcode: 0,
    }
}

/// A fake host: `connect` hands the session the guest end of a fresh
/// socket pair each time it is called; `serve` runs on the host end.
fn fake_host<F>(serve: F) -> Connect
where
    F: Fn(UnixStream) + Send + Sync + 'static,
{
    let serve = Arc::new(serve);
    Arc::new(move || {
        let (host, guest) = UnixStream::pair()?;
        let serve = Arc::clone(&serve);
        std::thread::spawn(move || serve(host));
        Ok(guest.into_raw_fd())
    })
}

fn echo_host() -> Connect {
    fake_host(|mut host| {
        while let Some(request) = read_frame(&mut host) {
            write_response(&mut host, &answered(&request));
        }
    })
}

#[tokio::test]
async fn sixty_four_concurrent_queries_answered_out_of_order_each_get_their_own_answer() {
    let connect = fake_host(|mut host| {
        // Collect a batch, answer it in reverse.
        let mut batch = Vec::new();
        while let Some(request) = read_frame(&mut host) {
            batch.push(request);
            if batch.len() == 64 {
                for request in batch.drain(..).rev() {
                    write_response(&mut host, &answered(&request));
                }
            }
        }
    });
    let forwarder = Arc::new(DnsForwarder::new(1, connect));
    let mut tasks = Vec::new();
    for i in 0..64u16 {
        let forwarder = Arc::clone(&forwarder);
        tasks.push(tokio::spawn(async move {
            let q = query(i, &format!("host{i}.example"));
            let response = forwarder.forward_query(q.clone(), "udp").await.unwrap();
            assert_eq!(
                &response.raw[0..2],
                &q[0..2],
                "answer carries this query's transaction id"
            );
            assert_eq!(&response.raw[12..], &q[12..], "answer carries this query's question");
        }));
    }
    for task in tasks {
        task.await.unwrap();
    }
}

#[tokio::test]
async fn answers_for_unknown_ids_or_other_questions_are_discarded_and_the_query_keeps_waiting() {
    let connect = fake_host(|mut host| {
        while let Some(request) = read_frame(&mut host) {
            // 1. an answer for an id nobody asked with
            write_response(
                &mut host,
                &DnsResponse {
                    id: request.id.wrapping_add(1000),
                    raw: answer_to(&request.raw),
                    decision: "allowed".into(),
                    rcode: 0,
                },
            );
            // 2. the right id, another question
            write_response(
                &mut host,
                &DnsResponse {
                    id: request.id,
                    raw: answer_to(&query(1, "attacker.example")),
                    decision: "allowed".into(),
                    rcode: 0,
                },
            );
            // 3. the right id and question, another transaction id
            write_response(
                &mut host,
                &DnsResponse {
                    id: request.id,
                    raw: answer_to(&query(0xFFFF, "victim.example")),
                    decision: "allowed".into(),
                    rcode: 0,
                },
            );
            // 4. the real answer
            write_response(&mut host, &answered(&request));
        }
    });
    let forwarder = DnsForwarder::new(1, connect);
    let q = query(42, "victim.example");
    let response = forwarder.forward_query(q.clone(), "udp").await.unwrap();
    assert_eq!(response.raw, answer_to(&q));
}

#[tokio::test]
async fn in_flight_cap_sheds_without_blocking() {
    let connect = fake_host(|mut host| {
        // Read everything, answer nothing.
        while read_frame(&mut host).is_some() {}
    });
    let forwarder = Arc::new(DnsForwarder::new(1, connect));
    let mut waiting = Vec::new();
    for i in 0..DNS_SESSION_MAX_IN_FLIGHT as u16 {
        let forwarder = Arc::clone(&forwarder);
        waiting.push(tokio::spawn(async move {
            forwarder.sessions[0]
                .forward_query_within(query(i, "slow.example"), "udp", Duration::from_secs(5))
                .await
        }));
    }
    // Let the spawned queries register.
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(forwarder.sessions[0].in_flight(), DNS_SESSION_MAX_IN_FLIGHT);
    let started = std::time::Instant::now();
    let shed = forwarder
        .forward_query(query(9999, "one-too-many.example"), "udp")
        .await;
    assert!(shed.is_err(), "the query past the cap must be refused");
    assert!(started.elapsed() < Duration::from_millis(500), "shedding must not wait");
    for task in waiting {
        task.abort();
    }
}

#[tokio::test]
async fn a_query_past_its_deadline_fails_and_frees_its_slot() {
    let connect = fake_host(|mut host| while read_frame(&mut host).is_some() {});
    let forwarder = DnsForwarder::new(1, connect);
    let started = std::time::Instant::now();
    let error = forwarder.sessions[0]
        .forward_query_within(query(1, "never.example"), "udp", Duration::from_millis(100))
        .await
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(forwarder.sessions[0].in_flight(), 0);
}

#[tokio::test]
async fn a_lost_connection_fails_every_pending_query_and_the_session_reconnects() {
    let first = Arc::new(AtomicBool::new(true));
    let connect = fake_host({
        let first = Arc::clone(&first);
        move |mut host| {
            if first.swap(false, Ordering::SeqCst) {
                // Take the first two queries and hang up on them.
                let _ = read_frame(&mut host);
                let _ = read_frame(&mut host);
                drop(host);
                return;
            }
            while let Some(request) = read_frame(&mut host) {
                write_response(&mut host, &answered(&request));
            }
        }
    });
    let forwarder = Arc::new(DnsForwarder::new(1, connect));
    let a = {
        let f = Arc::clone(&forwarder);
        tokio::spawn(async move { f.forward_query(query(1, "a.example"), "udp").await })
    };
    let b = {
        let f = Arc::clone(&forwarder);
        tokio::spawn(async move { f.forward_query(query(2, "b.example"), "udp").await })
    };
    assert!(a.await.unwrap().is_err(), "a query on the lost connection fails");
    assert!(b.await.unwrap().is_err(), "every query on the lost connection fails");
    assert_eq!(forwarder.sessions[0].in_flight(), 0);
    // The reconnected session answers.
    let q = query(3, "c.example");
    let response = tokio::time::timeout(Duration::from_secs(5), forwarder.forward_query(q.clone(), "udp"))
        .await
        .expect("session must reconnect")
        .unwrap();
    assert_eq!(response.raw, answer_to(&q));
}

#[tokio::test]
async fn an_oversized_response_frame_ends_the_connection_rather_than_the_process() {
    let first = Arc::new(AtomicBool::new(true));
    let connect = fake_host({
        let first = Arc::clone(&first);
        move |mut host| {
            if first.swap(false, Ordering::SeqCst) {
                let _ = read_frame(&mut host);
                host.write_all(&(MAX_FRAME_SIZE + 1).to_be_bytes()).unwrap();
                let _ = host.write_all(&[0u8; 64]);
                return;
            }
            while let Some(request) = read_frame(&mut host) {
                write_response(&mut host, &answered(&request));
            }
        }
    });
    let forwarder = DnsForwarder::new(1, connect);
    assert!(forwarder.forward_query(query(1, "big.example"), "udp").await.is_err());
    let q = query(2, "after.example");
    let response = tokio::time::timeout(Duration::from_secs(5), forwarder.forward_query(q.clone(), "udp"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(response.raw, answer_to(&q));
}

#[tokio::test]
async fn a_malformed_query_is_forwarded_and_its_empty_answer_delivered() {
    // The host answers a query it cannot parse with empty bytes; the
    // session has no question to check against and passes that through.
    let connect = fake_host(|mut host| {
        while let Some(request) = read_frame(&mut host) {
            write_response(
                &mut host,
                &DnsResponse {
                    id: request.id,
                    raw: Vec::new(),
                    decision: "error".into(),
                    rcode: 2,
                },
            );
        }
    });
    let forwarder = DnsForwarder::new(1, connect);
    let response = forwarder.forward_query(vec![1, 2, 3], "udp").await.unwrap();
    assert!(response.raw.is_empty());
}

#[tokio::test]
async fn correlation_ids_are_never_zero() {
    let (session, worker) = DnsSession::spawn(0, echo_host());
    session.next_id.store(u32::MAX, Ordering::Relaxed);
    assert_eq!(session.alloc_id(), u32::MAX);
    assert_eq!(session.alloc_id(), 1, "zero is the legacy 'no id' value and is skipped");
    worker.abort();
}

#[tokio::test]
async fn an_unparseable_query_cannot_receive_an_unverified_nonempty_answer() {
    let (session, _frames) = idle_session(1);
    let mut request = Box::pin(session.forward_query(vec![1, 2, 3], "udp"));
    std::future::poll_fn(|cx| {
        assert!(std::future::Future::poll(request.as_mut(), cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    let id = *session.pending.lock().unwrap().keys().next().unwrap();
    let mut response = DnsResponse {
        id,
        raw: answer_to(&query(1, "unverified.example")),
        decision: "error".into(),
        rcode: 1,
    };
    let frame = capsem_proto::encode_dns_response(&response).unwrap();
    session.deliver(&frame[4..]);
    assert_eq!(session.in_flight(), 1, "unverified bytes must not complete a query");
    response.raw.clear();
    let frame = capsem_proto::encode_dns_response(&response).unwrap();
    session.deliver(&frame[4..]);
    assert!(request.await.unwrap().raw.is_empty());
}
