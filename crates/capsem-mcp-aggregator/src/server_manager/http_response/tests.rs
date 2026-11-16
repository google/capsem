use super::*;
use futures::stream;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

fn response(chunks: Vec<&'static str>, length: Option<u64>) -> reqwest::Response {
    let body = reqwest::Body::wrap_stream(stream::iter(
        chunks.into_iter().map(|chunk| Ok::<_, io::Error>(Bytes::from(chunk))),
    ));
    let mut response = http::Response::builder();
    if let Some(length) = length {
        response = response.header("content-length", length);
    }
    reqwest::Response::from(response.body(body).unwrap())
}

#[tokio::test]
async fn a_body_at_the_limit_preserves_every_byte() {
    assert_eq!(
        body(response(vec!["abc", "", "defgh"], None), 8).await.unwrap(),
        b"abcdefgh"
    );
}

#[tokio::test]
async fn an_oversized_length_is_rejected_without_reading_the_body() {
    let polls = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&polls);
    let stream = stream::once(async move {
        observed.fetch_add(1, Ordering::Relaxed);
        Ok::<_, io::Error>(Bytes::from("x"))
    });
    let response = http::Response::builder()
        .header("content-length", 9)
        .body(reqwest::Body::wrap_stream(stream))
        .unwrap();
    assert!(body(reqwest::Response::from(response), 8).await.is_err());
    assert_eq!(polls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn streamed_body_limits_ignore_missing_and_dishonest_length_hints() {
    for length in [None, Some(1)] {
        let error = body(response(vec!["abcd", "efgh", "i"], length), 8).await.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("tool may already have run"));
    }
}

async fn events(chunks: Vec<&'static str>, limit: usize) -> Vec<Result<Sse, SseError>> {
    sse(
        stream::iter(chunks.into_iter().map(|chunk| Ok::<_, io::Error>(Bytes::from(chunk)))),
        limit,
    )
    .collect()
    .await
}

#[tokio::test]
async fn an_unterminated_sse_line_is_bounded_before_parsing() {
    let received = events(vec!["data:12345"], 8).await;
    assert!(matches!(received.as_slice(), [Err(_)]), "{received:?}");
}

#[tokio::test]
async fn multiline_data_and_intervening_comments_cannot_bypass_the_event_limit() {
    for chunks in [vec!["data:a\n", "data:b\n"], vec!["data:abc\n", ":ok\n", "data:x\n"]] {
        let received = events(chunks, 10).await;
        assert!(matches!(received.as_slice(), [Err(_)]), "{received:?}");
    }
}

#[tokio::test]
async fn event_boundaries_reset_the_limit_even_when_crlf_splits_across_chunks() {
    let received = events(vec!["data:1\r", "\n\r", "\n", "data:2\n\n"], 8).await;
    assert_eq!(received.len(), 2, "{received:?}");
    assert!(received.iter().all(Result::is_ok));
}

#[tokio::test]
async fn an_oversized_body_does_not_wait_for_eof() {
    let chunks = stream::iter([Ok::<_, io::Error>(Bytes::from("         "))]).chain(stream::pending());
    let response = http::Response::new(reqwest::Body::wrap_stream(chunks));
    let error = tokio::time::timeout(std::time::Duration::from_secs(1), body(response.into(), 8))
        .await
        .expect("reject before waiting for an unterminated HTTP body")
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
}

#[tokio::test]
async fn an_oversized_sse_event_drops_its_stream_without_another_read() {
    let chunks = stream::iter([Ok::<_, io::Error>(Bytes::from("data:12345"))]).chain(stream::pending());
    let mut events = sse(chunks, 8);
    let first = tokio::time::timeout(std::time::Duration::from_secs(1), events.next())
        .await
        .expect("reject the incomplete event before waiting for its terminator");
    assert!(first.unwrap().is_err());
    assert!(tokio::time::timeout(std::time::Duration::from_secs(1), events.next())
        .await
        .expect("the rejected upstream is no longer polled")
        .is_none());
}

#[tokio::test]
async fn valid_sse_content_survives_every_byte_split_including_utf8_and_crlf() {
    let input = b"data:\xc3\xa9\r\n\r\ndata:next\n\n";
    for split in 0..=input.len() {
        let chunks = [Bytes::from_static(&input[..split]), Bytes::from_static(&input[split..])];
        let received: Vec<_> = sse(stream::iter(chunks.map(Ok::<_, io::Error>)), 16).collect().await;
        assert_eq!(received.len(), 2, "split={split}: {received:?}");
        let values: Vec<_> = received.into_iter().map(Result::unwrap).collect();
        assert_eq!(values[0].data.as_deref(), Some("é"), "split={split}");
        assert_eq!(values[1].data.as_deref(), Some("next"), "split={split}");
        assert!(values
            .iter()
            .all(|event| event.event.is_none() && event.id.is_none() && event.retry.is_none()));
    }
}
