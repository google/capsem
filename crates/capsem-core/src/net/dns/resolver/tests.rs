//! The upstream forwarder must only accept the answer to the question it
//! asked. A UDP socket connected to the upstream still delivers any datagram
//! whose source address is spoofed to look like that upstream, and the
//! transaction id is the guest's to choose, so the only defence against a
//! forged answer -- and against caching it for five minutes -- is checking
//! the id and the question on every datagram and waiting past the ones that
//! do not match.

use super::*;
use hickory_proto::op::{Message, MessageType, OpCode, Query};
use hickory_proto::rr::{Name, RecordType};
use std::str::FromStr;

fn query_bytes(id: u16, name: &str) -> Vec<u8> {
    let mut message = Message::new(id, MessageType::Query, OpCode::Query);
    message.add_query(Query::query(Name::from_str(name).unwrap(), RecordType::A));
    message.metadata.recursion_desired = true;
    message.to_vec().unwrap()
}

fn answer_for(query: &[u8], id: u16, name: &str) -> Vec<u8> {
    let mut message = Message::from_vec(query).unwrap();
    message.metadata.id = id;
    message.metadata.message_type = MessageType::Response;
    message.queries.clear();
    message.add_query(Query::query(Name::from_str(name).unwrap(), RecordType::A));
    message.to_vec().unwrap()
}

/// A fake upstream that answers every query with `replies(query)` datagrams
/// in order, from the same socket the query arrived on.
async fn fake_upstream(replies: impl Fn(&[u8]) -> Vec<Vec<u8>> + Send + 'static) -> SocketAddr {
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = socket.local_addr().unwrap();
    tokio::spawn(async move {
        let mut buf = vec![0u8; 4096];
        while let Ok((n, peer)) = socket.recv_from(&mut buf).await {
            for reply in replies(&buf[..n]) {
                socket.send_to(&reply, peer).await.unwrap();
            }
        }
    });
    addr
}

fn resolver(upstream: SocketAddr) -> DnsResolver {
    DnsResolver::with_upstreams(vec![upstream]).with_timeout(Duration::from_millis(400))
}

#[tokio::test]
async fn a_forged_answer_with_another_id_is_skipped_for_the_real_one() {
    let upstream = fake_upstream(|query| {
        let id = u16::from_be_bytes([query[0], query[1]]);
        vec![
            answer_for(query, id ^ 0x5a5a, "example.com."),
            answer_for(query, id, "example.com."),
        ]
    })
    .await;
    let query = query_bytes(0x1234, "example.com.");
    let (answer, _) = resolver(upstream)
        .resolve(&query)
        .await
        .expect("the matching answer arrives");
    assert_eq!(&answer[..2], &query[..2], "the returned answer carries the query's id");
}

#[tokio::test]
async fn an_answer_to_a_different_question_is_not_accepted() {
    let upstream = fake_upstream(|query| {
        let id = u16::from_be_bytes([query[0], query[1]]);
        vec![answer_for(query, id, "attacker.example.")]
    })
    .await;
    let query = query_bytes(0x0042, "bank.example.");
    let error = resolver(upstream)
        .resolve(&query)
        .await
        .expect_err("an answer for another name must not be returned");
    assert!(
        error.to_string().contains("timeout") || error.to_string().contains("timed out"),
        "{error}"
    );
}

#[tokio::test]
async fn only_forged_answers_means_no_answer() {
    let upstream = fake_upstream(|query| {
        let id = u16::from_be_bytes([query[0], query[1]]);
        vec![answer_for(query, id.wrapping_add(1), "example.com."); 3]
    })
    .await;
    let query = query_bytes(0x0001, "example.com.");
    resolver(upstream)
        .resolve(&query)
        .await
        .expect_err("wrong ids must be discarded, not returned");
}

#[tokio::test]
async fn a_reflected_query_is_not_an_answer() {
    // Same id, same question, but the QR bit is clear: it is our own query
    // coming back, not a response.
    let upstream = fake_upstream(|query| vec![query.to_vec()]).await;
    let query = query_bytes(0x0007, "example.com.");
    resolver(upstream)
        .resolve(&query)
        .await
        .expect_err("a datagram without the response bit is not an answer");
}

#[tokio::test]
async fn a_truncated_datagram_is_discarded() {
    let upstream = fake_upstream(|query| {
        let id = u16::from_be_bytes([query[0], query[1]]);
        vec![vec![query[0], query[1], 0x81], answer_for(query, id, "example.com.")]
    })
    .await;
    let query = query_bytes(0x0100, "example.com.");
    let (answer, _) = resolver(upstream)
        .resolve(&query)
        .await
        .expect("the full answer arrives after the runt");
    assert!(answer.len() > 12);
}

#[tokio::test]
async fn an_upstream_that_never_answers_times_out_within_the_attempt_budget() {
    // A socket that swallows every query.
    let sink = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = sink.local_addr().unwrap();
    tokio::spawn(async move {
        let mut buf = [0u8; 512];
        while sink.recv_from(&mut buf).await.is_ok() {}
    });
    let resolver = DnsResolver::with_upstreams(vec![addr]).with_timeout(Duration::from_millis(200));
    let started = std::time::Instant::now();
    let error = resolver.resolve(&query_bytes(1, "slow.example.")).await.unwrap_err();
    let took = started.elapsed();
    assert!(
        took >= Duration::from_millis(200) && took < Duration::from_secs(2),
        "{took:?}: {error:#}"
    );
}

#[test]
fn the_default_attempt_budget_fits_two_upstreams_inside_the_guest_resolver_timeout() {
    // glibc's RES_TIMEOUT is 5 s. Two sequential attempts must finish
    // inside it or the client retransmits while we are still waiting.
    assert!(DEFAULT_TIMEOUT * DEFAULT_UPSTREAMS.len() as u32 <= Duration::from_secs(5));
}

#[test]
fn responses_must_match_the_operation_and_have_exactly_one_question() {
    let query = query_bytes(7, "example.com.");
    let expected = ExpectedAnswer::for_query(&query).unwrap();
    let answer = answer_for(&query, 7, "example.com.");
    assert!(expected.check(&answer).is_ok());
    let mut wrong_opcode = answer.clone();
    wrong_opcode[2] |= 5 << 3;
    assert!(expected.check(&wrong_opcode).is_err());
    let mut extra = Message::from_vec(&answer).unwrap();
    extra.add_query(Query::query(
        Name::from_str("attacker.example.").unwrap(),
        RecordType::A,
    ));
    assert!(expected.check(&extra.to_vec().unwrap()).is_err());
}

#[tokio::test]
async fn a_hundred_concurrent_resolves_complete_against_one_upstream() {
    let upstream = fake_upstream(|query| {
        let id = u16::from_be_bytes([query[0], query[1]]);
        let name = Message::from_vec(query).unwrap().queries[0].name().to_string();
        vec![answer_for(query, id, &name)]
    })
    .await;
    let resolver = std::sync::Arc::new(resolver(upstream));
    let mut tasks = Vec::new();
    for i in 0..100u16 {
        let resolver = std::sync::Arc::clone(&resolver);
        tasks.push(tokio::spawn(async move {
            let name = format!("h{i}.example.");
            let (answer, _) = resolver.resolve(&query_bytes(i, &name)).await.unwrap();
            assert_eq!(u16::from_be_bytes([answer[0], answer[1]]), i);
            assert_eq!(Message::from_vec(&answer).unwrap().queries[0].name().to_string(), name);
        }));
    }
    for task in tasks {
        task.await.unwrap();
    }
}
