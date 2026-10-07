//! Bounded shared WebSocket fixture transports, including Responses turns.
use super::{full, log_request, response, responses, RespBody, State};
use crate::limits::{read_ws_frame, write_ws_frame, MAX_WS_FRAME_BYTES};
use base64::Engine as _;
use bytes::Bytes;
use hyper::body::Incoming;
use hyper::header::{CONNECTION, SEC_WEBSOCKET_ACCEPT, SEC_WEBSOCKET_KEY, UPGRADE};
use hyper::upgrade::Upgraded;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde_json::{json, Value};
use sha1::{Digest, Sha1};
use std::sync::atomic::{AtomicU64, Ordering};

pub(super) async fn upgrade(mut req: Request<Incoming>, path: String, state: State) -> Response<RespBody> {
    let key = req
        .headers()
        .get(SEC_WEBSOCKET_KEY)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    if key.is_empty() {
        return response(StatusCode::BAD_REQUEST, Bytes::new(), "text/plain");
    }
    let accept = accept(key);
    let headers = req.headers().clone();
    let on_upgrade = hyper::upgrade::on(&mut req);
    tokio::spawn(async move {
        if let Ok(upgraded) = on_upgrade.await {
            let _ = serve(TokioIo::new(upgraded), path, state, headers).await;
        }
    });
    Response::builder()
        .status(StatusCode::SWITCHING_PROTOCOLS)
        .header(UPGRADE, "websocket")
        .header(CONNECTION, "Upgrade")
        .header(SEC_WEBSOCKET_ACCEPT, accept)
        .body(full(Bytes::new()))
        .expect("build websocket upgrade")
}

pub(super) fn accept(key: &str) -> String {
    let mut hasher = Sha1::new();
    hasher.update(key.as_bytes());
    hasher.update(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
    base64::engine::general_purpose::STANDARD.encode(hasher.finalize())
}

async fn serve(mut io: TokioIo<Upgraded>, path: String, state: State, headers: hyper::HeaderMap) -> anyhow::Result<()> {
    if path == "/ws/close" {
        return write_ws_frame(&mut io, 8, &[]).await;
    }
    if path == "/ws/ping" {
        return write_ws_frame(&mut io, 9, b"capsem-ping").await;
    }
    let mut previous = None;
    while let Some((opcode, payload)) = read_ws_frame(&mut io).await? {
        match opcode {
            1 if path == "/v1/responses" => {
                let result = serde_json::from_slice::<Value>(&payload)
                    .map_err(|error| format!("Invalid JSON: {error}"))
                    .and_then(|request| create(request, &mut previous));
                let (status, events) = match result {
                    Ok(message) => (StatusCode::OK, responses::events(message)),
                    Err(message) => (
                        StatusCode::BAD_REQUEST,
                        vec![json!({"type": "error", "status": 400,
                        "error": {"type": "invalid_request_error", "code": "invalid_request", "message": message, "param": null}})],
                    ),
                };
                let mut body = Vec::new();
                for event in events {
                    let bytes = serde_json::to_vec(&event)?;
                    write_ws_frame(&mut io, 1, &bytes).await?;
                    body.extend_from_slice(&bytes);
                    body.push(b'\n');
                }
                log_request(
                    &state,
                    &Method::GET,
                    &path,
                    None,
                    &headers,
                    &payload,
                    &response(status, Bytes::from(body), "application/x-ndjson"),
                );
            }
            1 | 2 if path != "/v1/responses" => write_ws_frame(&mut io, opcode, &payload).await?,
            8 => {
                write_ws_frame(&mut io, 8, &payload).await?;
                break;
            }
            9 => write_ws_frame(&mut io, 10, &payload).await?,
            _ => {}
        }
    }
    Ok(())
}

/// Retain only the last accepted turn on this connection, bounded like a frame.
fn create(mut request: Value, previous: &mut Option<(Value, Value)>) -> Result<Value, String> {
    if request["type"] != "response.create" {
        return Err("Expected response.create".into());
    }
    let input = match request.get("input") {
        Some(Value::Array(items)) => items.clone(),
        Some(Value::String(text)) => vec![json!({"role": "user", "content": text})],
        None => Vec::new(),
        _ => return Err("Expected input items or text".into()),
    };
    if let Some(id) = request.get("previous_response_id").filter(|id| !id.is_null()) {
        let (prior, output) = previous
            .as_ref()
            .filter(|(_, output)| output["id"] == *id)
            .ok_or("previous_response_id not found on this connection")?;
        let mut accumulated = prior["input"].as_array().unwrap().clone();
        accumulated.extend(output["output"].as_array().unwrap().iter().cloned());
        accumulated.extend(input);
        request["input"] = json!(accumulated);
        for key in ["tools", "model", "instructions"] {
            if request.get(key).is_none() {
                request[key] = prior[key].clone();
            }
        }
    } else {
        request["input"] = json!(input);
    }
    if request.to_string().len() as u64 > MAX_WS_FRAME_BYTES {
        return Err("Response context exceeds the fixture frame limit".into());
    }
    let mut output = responses::message(&request)?;
    if request["generate"] == false {
        output["output"] = json!([]);
        output["usage"] = Value::Null;
    }
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    output["id"] = json!(format!("resp_capsem_ws_{}", NEXT_ID.fetch_add(1, Ordering::Relaxed)));
    *previous = Some((request, output.clone()));
    Ok(output)
}
