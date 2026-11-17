use super::*;

#[tokio::test]
async fn refused_post_sse_is_correlated_local_error_and_drops_without_another_read() {
    let limit = capsem_proto::MCP_PAYLOAD_MAX_BYTES;
    let data = format!(
        r#"data: {{"jsonrpc":"2.0","id":7,"result":"{}"}}

"#,
        "x".repeat(limit)
    );
    let chunks = stream::iter([Ok::<_, io::Error>(Bytes::from(data))]).chain(stream::pending());
    let mut received = post_sse(
        chunks,
        capsem_proto::MCP_JSON_RPC_MAX_BYTES,
        Some(rmcp::model::RequestId::Number(7)),
    );
    let event = received.next().await.unwrap().unwrap();
    let message: serde_json::Value = serde_json::from_str(event.data.as_deref().unwrap()).unwrap();
    assert_eq!(message["id"], 7);
    assert!(message.get("result").is_none());
    assert!(message["error"]["message"]
        .as_str()
        .unwrap()
        .contains("tool may already have run"));
    assert_eq!(message["error"]["data"]["origin"], "capsem_transport");
    assert!(tokio::time::timeout(std::time::Duration::from_secs(1), received.next())
        .await
        .expect("a refused POST stream must not wait for another upstream item")
        .is_none());
}

#[tokio::test]
async fn correlation_envelope_is_shared_by_json_and_sse_without_content_credit() {
    use capsem_proto::mcp_contracts::{json_rpc_wire::encode_response, JsonRpcResponse};
    let limit = capsem_proto::MCP_PAYLOAD_MAX_BYTES;
    let empty = JsonRpcResponse::ok(Some(serde_json::json!(1)), serde_json::json!(""));
    let overhead = serde_json::to_vec(&empty).unwrap().len() - b",\"id\":1".len();
    let mut message = JsonRpcResponse::ok(
        Some(serde_json::json!("\0".repeat(capsem_proto::MCP_REQUEST_ID_MAX_BYTES))),
        serde_json::json!("x".repeat(limit - overhead)),
    );
    let accepted = encode_response(&message).unwrap();
    let response = http::Response::new(reqwest::Body::from(accepted.clone()));
    assert_eq!(json_body(response.into()).await.unwrap(), accepted);
    let mut input = b"data: ".to_vec();
    input.extend_from_slice(&accepted);
    input.extend_from_slice(b"\n\n");
    let mut received = sse(
        stream::iter([Ok::<_, io::Error>(Bytes::from(input))]),
        capsem_proto::MCP_JSON_RPC_MAX_BYTES,
    );
    assert_eq!(
        received.next().await.unwrap().unwrap().data.as_deref(),
        Some(std::str::from_utf8(&accepted).unwrap())
    );
    message.id = Some(serde_json::json!(1));
    message.result = Some(serde_json::json!("x".repeat(limit - overhead + 1)));
    let oversized = serde_json::to_vec(&message).unwrap();
    let response = http::Response::new(reqwest::Body::from(oversized.clone()));
    assert!(json_body(response.into()).await.is_err());
    let mut input = b"data: ".to_vec();
    input.extend_from_slice(&oversized);
    input.extend_from_slice(b"\n\n");
    let mut received = sse(
        stream::iter([Ok::<_, io::Error>(Bytes::from(input))]),
        capsem_proto::MCP_JSON_RPC_MAX_BYTES,
    );
    assert!(
        received.next().await.unwrap().is_err(),
        "unused ID allowance cannot become content credit"
    );
}
use futures::stream;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

#[tokio::test]
async fn usable_sse_payload_excludes_prefixes_line_endings_and_metadata() {
    for input in [
        "data: éé\r\n\r\n",
        "event:x\nid: 7\ndata: éé\n\n",
        "data:a\ndata:bc\n\n",
    ] {
        let expected = if input.contains("é") { "éé" } else { "a\nbc" };
        for split in 0..=input.len() {
            let bytes = Bytes::copy_from_slice(input.as_bytes());
            let chunks = [bytes.slice(..split), bytes.slice(split..)];
            let received: Vec<_> = sse(stream::iter(chunks.map(Ok::<_, io::Error>)), 4).collect().await;
            assert_eq!(received.len(), 1, "split {split}: {received:?}");
            assert_eq!(
                received.into_iter().next().unwrap().unwrap().data.as_deref(),
                Some(expected)
            );
        }
    }
}

#[tokio::test]
async fn initial_bom_empty_data_lines_and_keepalives_preserve_the_exact_payload() {
    for (input, expected) in [
        ("\u{feff}data: éé\r\n\r\n", "éé"),
        ("data:\n:keep\ndata:\n:keep\ndata:\ndata:\ndata:\n\n", "\n\n\n\n"),
    ] {
        for split in 0..=input.len() {
            let bytes = Bytes::copy_from_slice(input.as_bytes());
            let chunks = [bytes.slice(..split), bytes.slice(split..)];
            let received: Vec<_> = sse(stream::iter(chunks.map(Ok::<_, io::Error>)), 4).collect().await;
            assert_eq!(received.len(), 1, "split {split}: {received:?}");
            assert_eq!(
                received.into_iter().next().unwrap().unwrap().data.as_deref(),
                Some(expected)
            );
        }
    }
}

#[tokio::test]
async fn shared_payload_boundary_has_the_same_usable_bytes_in_http_and_sse() {
    let limit = capsem_proto::MCP_PAYLOAD_MAX_BYTES;
    let data = "x".repeat(limit);
    let input = Bytes::from(format!("data: {data}\r\n\r\n"));
    let chunks: Vec<_> = (0..input.len())
        .step_by(8192)
        .map(|start| Ok::<_, io::Error>(input.slice(start..(start + 8192).min(input.len()))))
        .collect();
    let mut received = sse(stream::iter(chunks), limit);
    assert_eq!(
        received.next().await.unwrap().unwrap().data.as_deref(),
        Some(data.as_str())
    );
    assert!(received.next().await.is_none());
    let response = http::Response::new(reqwest::Body::from(data));
    assert_eq!(body(response.into(), limit).await.unwrap().len(), limit);
}

#[tokio::test]
async fn comment_and_metadata_lines_remain_bounded_without_a_terminator() {
    for input in [":xxxxxxxxxxx", "id:xxxxxxxxx"] {
        let chunks = stream::iter([Ok::<_, io::Error>(Bytes::from_static(input.as_bytes()))]).chain(stream::pending());
        let mut received = sse(chunks, 4);
        assert!(tokio::time::timeout(std::time::Duration::from_secs(1), received.next())
            .await
            .unwrap()
            .unwrap()
            .is_err());
    }
}

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
    let received = events(vec!["data:123456789"], 8).await;
    assert!(matches!(received.as_slice(), [Err(_)]), "{received:?}");
}

#[tokio::test]
async fn multiline_data_and_intervening_comments_cannot_bypass_the_event_limit() {
    for chunks in [vec!["data:a\n", "data:b\n"], vec!["data:abc\n", ":ok\n", "data:x\n"]] {
        let received = events(chunks, 2).await;
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
    let chunks = stream::iter([Ok::<_, io::Error>(Bytes::from("data:123456789"))]).chain(stream::pending());
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
