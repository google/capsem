//! Real upgrade and masked-frame proof for the Responses fixture.
use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const TOKEN: &str = "0123456789abcdef0123456789abcdef";

async fn connect() -> TcpStream {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(serve_http(
        listener,
        State {
            request_log: None,
            dns_answers: DnsAnswers::default(),
        },
        false,
    ));
    let mut stream = TcpStream::connect(addr).await.unwrap();
    stream.write_all(b"GET /v1/responses HTTP/1.1\r\nHost: mock\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n").await.unwrap();
    let mut header = Vec::new();
    while !header.ends_with(b"\r\n\r\n") {
        header.push(stream.read_u8().await.unwrap());
        assert!(header.len() < 4096);
    }
    assert!(
        header.starts_with(b"HTTP/1.1 101"),
        "{}",
        String::from_utf8_lossy(&header)
    );
    assert!(String::from_utf8_lossy(&header).contains("s3pPLMBiTxaQ9kYGzzhZRbK+xOo="));
    stream
}

async fn send(stream: &mut TcpStream, opcode: u8, bytes: &[u8]) {
    let mask = [1, 2, 3, 4];
    let mut frame = vec![0x80 | opcode];
    if bytes.len() < 126 {
        frame.push(0x80 | u8::try_from(bytes.len()).unwrap());
    } else {
        frame.push(0x80 | 126);
        frame.extend_from_slice(&u16::try_from(bytes.len()).unwrap().to_be_bytes());
    }
    frame.extend_from_slice(&mask);
    frame.extend(bytes.iter().enumerate().map(|(index, byte)| byte ^ mask[index % 4]));
    stream.write_all(&frame).await.unwrap();
}

async fn receive(stream: &mut TcpStream) -> (u8, Vec<u8>) {
    tokio::time::timeout(Duration::from_secs(5), async {
        let opcode = stream.read_u8().await.unwrap();
        let length = stream.read_u8().await.unwrap();
        assert_eq!(length & 0x80, 0, "server frames are unmasked");
        let size = match length {
            126 => usize::from(stream.read_u16().await.unwrap()),
            127 => usize::try_from(stream.read_u64().await.unwrap()).unwrap(),
            value => usize::from(value),
        };
        assert!(size <= 65536);
        let mut body = vec![0; size];
        stream.read_exact(&mut body).await.unwrap();
        (opcode & 0x0f, body)
    })
    .await
    .expect("bounded fixture event")
}

async fn turn(stream: &mut TcpStream, request: Value) -> Value {
    send(stream, 1, request.to_string().as_bytes()).await;
    for index in 0..32 {
        let (opcode, body) = receive(stream).await;
        assert_eq!(opcode, 1);
        let event: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(event["sequence_number"], index);
        if index == 0 {
            assert_eq!(event["type"], "response.created");
        }
        if event["type"] == "response.completed" {
            return event["response"].clone();
        }
    }
    panic!("missing completion");
}

fn proof() -> Value {
    json!({"type": "response.create", "model": "codex-fixture", "input": [
        {"role": "user", "content": format!("CAPSEM_MCP_PROOF={TOKEN}. Call echo.")}
    ], "tools": [{"type": "function", "name": "mcp__capsem__local__echo", "parameters": {
        "type": "object", "properties": {"text": {"type": "string"}}, "required": ["text"]
    }}]})
}

#[tokio::test]
async fn websocket_responses_complete_mcp_from_incremental_tool_result() {
    let mut stream = connect().await;
    let first = turn(&mut stream, proof()).await;
    assert_eq!(first["output"][0]["name"], "mcp__capsem__local__echo");
    assert_eq!(first["output"][0]["call_id"], "call_capsem_mcp_echo");
    let second = turn(
        &mut stream,
        json!({"type": "response.create", "model": "codex-fixture",
        "previous_response_id": first["id"], "input": [{"type": "function_call_output",
        "call_id": first["output"][0]["call_id"], "output": TOKEN}]}),
    )
    .await;
    assert_ne!(first["id"], second["id"]);
    assert_eq!(second["status"], "completed");
    assert_eq!(second["model"], "codex-fixture");
    assert_eq!(second["output"][0]["content"][0]["text"], TOKEN);
    send(&mut stream, 9, b"alive").await;
    assert_eq!(receive(&mut stream).await, (10, b"alive".to_vec()));
    send(&mut stream, 8, &1000_u16.to_be_bytes()).await;
    assert_eq!(receive(&mut stream).await.0, 8);
}

#[tokio::test]
async fn websocket_responses_reject_errors_without_echoing_or_poisoning_the_connection() {
    let mut stream = connect().await;
    for bytes in [
        b"not json".to_vec(),
        json!({"type": "unexpected"}).to_string().into_bytes(),
        json!({"type": "response.create", "previous_response_id": "unknown", "input": []})
            .to_string()
            .into_bytes(),
        {
            let mut payload = proof();
            payload["tools"] = json!([]);
            payload.to_string().into_bytes()
        },
    ] {
        send(&mut stream, 1, &bytes).await;
        let (opcode, body) = receive(&mut stream).await;
        let event: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(opcode, 1);
        assert_eq!(event["type"], "error");
        assert_eq!(event["status"], 400);
        assert_eq!(event["error"]["type"], "invalid_request_error");
        assert!(event["error"]["message"].as_str().is_some_and(|text| !text.is_empty()));
    }
    let first = turn(&mut stream, proof()).await;
    send(
        &mut stream,
        1,
        json!({"type": "response.create", "previous_response_id": first["id"],
        "input": [{"type": "function_call_output", "call_id": "call_capsem_mcp_echo", "output": "wrong"}]})
        .to_string()
        .as_bytes(),
    )
    .await;
    let (_, body) = receive(&mut stream).await;
    assert_eq!(serde_json::from_slice::<Value>(&body).unwrap()["type"], "error");
    let final_turn = turn(
        &mut stream,
        json!({"type": "response.create", "previous_response_id": first["id"],
        "input": [{"type": "function_call_output", "call_id": "call_capsem_mcp_echo", "output": TOKEN}]}),
    )
    .await;
    assert_eq!(final_turn["output"][0]["content"][0]["text"], TOKEN);
}

#[tokio::test]
async fn websocket_responses_warmup_preserves_context_without_generating_a_tool() {
    let mut stream = connect().await;
    let mut request = proof();
    request["generate"] = json!(false);
    let warmed = turn(&mut stream, request).await;
    assert_eq!(warmed["output"], json!([]));
    let generated = turn(
        &mut stream,
        json!({"type": "response.create", "previous_response_id": warmed["id"], "input": []}),
    )
    .await;
    assert_eq!(generated["output"][0]["name"], "mcp__capsem__local__echo");
}
