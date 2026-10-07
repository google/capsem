use super::*;
use crate::net::ai_traffic::{pricing::PricingTable, TraceState};
use crate::net::policy_config::{SecurityRuleProfile, SecurityRuleSet, SecurityRuleSource};
use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use tokio::io::DuplexStream;
use tokio_tungstenite::{
    tungstenite::{protocol::Role, Message},
    WebSocketStream,
};

type Socket = WebSocketStream<DuplexStream>;

fn context(db: Arc<DbWriter>, rules: &str) -> Context {
    let profile = SecurityRuleProfile::parse_toml(rules).unwrap();
    let rules = SecurityRuleSet::compile_profile(&profile, SecurityRuleSource::Corp).unwrap();
    let telemetry = Arc::new(telemetry_hook::TelemetryDeps {
        db,
        pricing: Arc::new(PricingTable::load()),
        trace_state: Arc::new(Mutex::new(TraceState::new())),
        security_rules: Arc::new(std::sync::RwLock::new(Arc::new(rules))),
        plugin_policy: Arc::new(std::sync::RwLock::new(BTreeMap::new().into())),
    });
    let policy = Arc::new(std::sync::RwLock::new(Arc::new(NetworkMechanics::new())));
    Context {
        pipeline: make_production_pipeline(policy, Arc::clone(&telemetry)),
        telemetry,
        conn: ConnMeta {
            domain: "api.openai.com".into(),
            process_name: Some("codex".into()),
            port: 443,
            protocol: Protocol::Tls,
            ai_provider: Some(ProviderKind::OpenAi),
            ai_protocol: Some(ModelProtocol::OpenAi),
        },
        ip: None,
        method: "GET".into(),
        path: "/v1/responses".into(),
        query: None,
        request_headers: "upgrade: websocket".into(),
        response_headers: "upgrade: websocket".into(),
        credential_ref: None,
        credential_observations: vec![],
    }
}

const ALLOW: &str =
    "[corp.rules.allow]\nname = 'allow'\naction = 'allow'\npriority = -10\nmatch = 'model.provider == \"openai\"'\n";

async fn sockets(context: Context) -> (Socket, Socket, tokio::task::JoinHandle<anyhow::Result<()>>) {
    let (client, guest) = tokio::io::duplex(65536);
    let (proxy, server) = tokio::io::duplex(65536);
    let bridge = tokio::spawn(bridge(guest, proxy, context));
    let client = WebSocketStream::from_raw_socket(client, Role::Client, None).await;
    let server = WebSocketStream::from_raw_socket(server, Role::Server, None).await;
    (client, server, bridge)
}

async fn receive(socket: &mut Socket) -> Value {
    let frame = tokio::time::timeout(std::time::Duration::from_secs(3), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    serde_json::from_str(frame.to_text().unwrap()).unwrap()
}

async fn exchange(client: &mut Socket, server: &mut Socket, request: Value, id: &str, text: &str) {
    client.send(Message::Text(request.to_string().into())).await.unwrap();
    assert_eq!(receive(server).await, request);
    let events = [
        json!({"type": "response.created", "response": {"id": id, "model": "gpt-fixture"}}),
        json!({"type": "response.output_text.delta", "delta": text}),
        json!({"type": "response.completed", "response": {"id": id, "model": "gpt-fixture", "status": "completed",
            "output": [{"type": "message", "content": [{"type": "output_text", "text": text}]}],
            "usage": {"input_tokens": 31, "output_tokens": 17, "total_tokens": 48}}}),
    ];
    for event in &events {
        server.send(Message::Text(event.to_string().into())).await.unwrap();
    }
    for event in events {
        assert_eq!(receive(client).await, event);
    }
}

#[tokio::test]
async fn websocket_models_keep_real_json_caller_usage_and_completion_without_counting_warmup() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let db = Arc::new(DbWriter::open(&path, 16).unwrap());
    let (mut client, mut server, job) = sockets(context(Arc::clone(&db), ALLOW)).await;
    exchange(
        &mut client,
        &mut server,
        json!({"type": "response.create", "model": "gpt-fixture", "generate": false, "input": []}),
        "warm",
        "",
    )
    .await;
    exchange(&mut client, &mut server, json!({"type": "response.create", "model": "gpt-fixture", "previous_response_id": "warm", "input": [{"role": "user", "content": "token"}]}), "first", "token").await;
    exchange(&mut client, &mut server, json!({"type": "response.create", "model": "gpt-fixture", "previous_response_id": "first", "input": [{"type": "function_call_output", "call_id": "call_echo", "output": "token"}]}), "final", "token").await;
    client.close(None).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(3), job)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    db.flush().await;
    let ledger = rusqlite::Connection::open(&path).unwrap();
    let rows: Vec<(String,String,String,i64,i64,String)> = ledger.prepare("SELECT provider,process_name,text_content,input_tokens,output_tokens,stop_reason FROM model_calls ORDER BY id").unwrap()
        .query_map([], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).unwrap().map(Result::unwrap).collect();
    assert_eq!(
        rows,
        vec![
            (
                "openai".into(),
                "codex".into(),
                "token".into(),
                31,
                17,
                "end_turn".into()
            );
            2
        ]
    );
    assert_eq!(
        ledger
            .query_row("SELECT COUNT(*) FROM net_events", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        3
    );
    db.shutdown_blocking();
}

#[tokio::test]
async fn websocket_models_enforce_live_request_rules_before_forwarding() {
    let dir = tempfile::tempdir().unwrap();
    let db = Arc::new(DbWriter::open(&dir.path().join("session.db"), 16).unwrap());
    let ctx = context(Arc::clone(&db), ALLOW);
    let rules = Arc::clone(&ctx.telemetry.security_rules);
    let (mut client, mut server, job) = sockets(ctx).await;
    exchange(
        &mut client,
        &mut server,
        json!({"type":"response.create", "model":"gpt-fixture", "input": []}),
        "one",
        "allowed",
    )
    .await;
    let profile = SecurityRuleProfile::parse_toml(
        "[corp.rules.block]\nname='block'\naction='block'\npriority=-100\nmatch='model.provider == \"openai\"'\n",
    )
    .unwrap();
    *rules.write().unwrap() = Arc::new(SecurityRuleSet::compile_profile(&profile, SecurityRuleSource::Corp).unwrap());
    client
        .send(Message::Text(
            json!({"type":"response.create","model":"gpt-fixture","input":[]})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    let denied = receive(&mut client).await;
    assert_eq!(denied["type"], "error");
    assert_eq!(denied["status"], 403);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(50), server.next())
            .await
            .is_err(),
        "denied request reached upstream"
    );
    client.close(None).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(3), job)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    db.shutdown_blocking();
}

#[tokio::test]
async fn websocket_models_block_completed_output_before_any_text_reaches_the_guest() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let db = Arc::new(DbWriter::open(&path, 16).unwrap());
    let rules = format!("{ALLOW}\n[corp.rules.secret]\nname='secret'\naction='block'\npriority=-100\nmatch='http.body.contains(\"SECRET\")'\n");
    let (mut client, mut server, job) = sockets(context(Arc::clone(&db), &rules)).await;
    let request = json!({"type":"response.create","model":"gpt-fixture","input":[]});
    client.send(Message::Text(request.to_string().into())).await.unwrap();
    assert_eq!(receive(&mut server).await, request);
    for event in [
        json!({"type":"response.created","response":{"id":"secret","model":"gpt-fixture"}}),
        json!({"type":"response.output_text.delta","delta":"SECRET"}),
        json!({"type":"response.completed","response":{"id":"secret","status":"completed","output":[]}}),
    ] {
        server.send(Message::Text(event.to_string().into())).await.unwrap();
    }
    let denied = receive(&mut client).await;
    assert_eq!(denied["type"], "error");
    assert_eq!(denied["status"], 403);
    assert!(!denied.to_string().contains("SECRET"));
    client.close(None).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(3), job)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    db.flush().await;
    let ledger = rusqlite::Connection::open(&path).unwrap();
    let row: (i64, Option<String>) = ledger
        .query_row("SELECT status_code,text_content FROM model_calls", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!(row.0, 403);
    assert!(row.1.is_none_or(|text| !text.contains("SECRET")));
    assert_eq!(
        ledger
            .query_row("SELECT policy_action FROM net_events", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "block"
    );
    let id = ledger
        .query_row("SELECT event_id FROM model_calls", [], |r| r.get::<_, String>(0))
        .unwrap();
    let archived = capsem_logger::DbHandle::open_external_reader(&path)
        .unwrap()
        .read_body(&id, "security_rule_events", capsem_logger::BodyDirection::Payload)
        .await
        .unwrap()
        .unwrap();
    let payload = capsem_proto::forensic::SecurityForensicEvent::decode(&archived.bytes)
        .unwrap()
        .to_json()
        .unwrap();
    let event: Value = serde_json::from_str(&payload).unwrap();
    assert_eq!(event["decision"]["effective"], "block");
    assert_eq!(event["process"]["name"], "codex");
    db.shutdown_blocking();
}

#[tokio::test]
async fn websocket_models_refuse_oversized_frames_before_body_allocation() {
    let dir = tempfile::tempdir().unwrap();
    let db = Arc::new(DbWriter::open(&dir.path().join("session.db"), 16).unwrap());
    let ctx = context(Arc::clone(&db), ALLOW);
    let (mut client, proxy) = tokio::io::duplex(1024);
    let (upstream, _server) = tokio::io::duplex(1024);
    let job = tokio::spawn(bridge(proxy, upstream, ctx));
    use tokio::io::AsyncWriteExt;
    // A masked frame header declaring one byte beyond the configured cap.
    // No body is sent or allocated by the test.
    let mut header = vec![0x81, 0x80 | 127];
    header.extend_from_slice(&u64::try_from(AI_BODY_CAPTURE_LIMIT + 1).unwrap().to_be_bytes());
    header.extend_from_slice(&[1, 2, 3, 4]);
    client.write_all(&header).await.unwrap();
    let failure = tokio::time::timeout(std::time::Duration::from_secs(3), job)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(failure.to_string().contains("Message too long"), "{failure}");
    db.shutdown_blocking();
}

#[tokio::test]
async fn websocket_models_finalize_an_interrupted_turn_and_reject_an_unmatched_response() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.db");
    let db = Arc::new(DbWriter::open(&path, 16).unwrap());
    let (mut client, mut server, job) = sockets(context(Arc::clone(&db), ALLOW)).await;
    let request = json!({"type":"response.create","model":"gpt-fixture","input":[]});
    client.send(Message::Text(request.to_string().into())).await.unwrap();
    assert_eq!(receive(&mut server).await, request);
    server
        .send(Message::Text(
            json!({"type":"response.created","response":{"id":"first"}})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    server
        .send(Message::Text(
            json!({"type":"response.completed","response":{"id":"wrong"}})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    let failure = tokio::time::timeout(std::time::Duration::from_secs(3), job)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(failure.to_string().contains("ID changed"));
    db.flush().await;
    let ledger = rusqlite::Connection::open(path).unwrap();
    assert_eq!(
        ledger
            .query_row("SELECT COUNT(*) FROM model_calls", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        ledger
            .query_row("SELECT stop_reason FROM model_calls", [], |r| r
                .get::<_, Option<String>>(0))
            .unwrap(),
        None
    );
    db.shutdown_blocking();
}

#[tokio::test]
async fn websocket_models_assemble_fragmented_json_and_preserve_ping_and_close() {
    use tokio_tungstenite::tungstenite::protocol::frame::{
        coding::{Data, OpCode},
        Frame,
    };
    let dir = tempfile::tempdir().unwrap();
    let db = Arc::new(DbWriter::open(&dir.path().join("session.db"), 16).unwrap());
    let (mut client, mut server, job) = sockets(context(Arc::clone(&db), ALLOW)).await;
    let request = json!({"type":"response.create","model":"gpt-fixture","input":"héllo"}).to_string();
    let cut = request.find('é').unwrap() + 1;
    client
        .send(Message::Frame(Frame::message(
            request.as_bytes()[..cut].to_vec(),
            OpCode::Data(Data::Text),
            false,
        )))
        .await
        .unwrap();
    client
        .send(Message::Frame(Frame::message(
            request.as_bytes()[cut..].to_vec(),
            OpCode::Data(Data::Continue),
            true,
        )))
        .await
        .unwrap();
    assert_eq!(
        receive(&mut server).await,
        serde_json::from_str::<Value>(&request).unwrap()
    );
    client.send(Message::Ping(b"alive".to_vec().into())).await.unwrap();
    let ping = tokio::time::timeout(std::time::Duration::from_secs(3), server.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(ping, Message::Ping(b"alive".to_vec().into()));
    server.send(Message::Pong(b"alive".to_vec().into())).await.unwrap();
    let pong = tokio::time::timeout(std::time::Duration::from_secs(3), client.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(pong, Message::Pong(b"alive".to_vec().into()));
    client.close(None).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(3), job)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    db.shutdown_blocking();
}
