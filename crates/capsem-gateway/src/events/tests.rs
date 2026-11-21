use super::*;
use axum::{body::Body, extract::State, response::Response, routing::get, Json, Router};
use bytes::Bytes;
use futures::StreamExt as _;
use serde_json::{json, Value};
use std::{
    convert::Infallible,
    net::SocketAddr,
    sync::atomic::{AtomicUsize, Ordering},
};
use tokio::sync::{mpsc, Mutex};

#[tokio::test]
async fn relay_preserves_partial_and_coalesced_records_and_rejects_a_truncated_tail() {
    let id = [0x5a; 16];
    let frame = SessionEvent::ApprovalPending(id).encode();
    let chunks = [frame[..2].to_vec(), [&frame[2..], &frame, &frame[..1]].concat()];
    let body = Body::from_stream(futures::stream::iter(chunks.map(Ok::<_, Infallible>)));
    let (tx, mut rx) = broadcast::channel(4);
    assert_eq!(
        relay(body, &tx).await.unwrap_err().to_string(),
        "truncated gateway event frame"
    );
    for _ in 0..2 {
        let Event::Message(text) = rx.try_recv().unwrap() else {
            panic!("expected session event")
        };
        assert_eq!(
            serde_json::from_str::<Value>(&text).unwrap(),
            json!({"type":"approval-pending","payload":{"id":uuid::Uuid::from_bytes(id).to_string()}})
        );
    }
    assert!(matches!(rx.try_recv(), Err(broadcast::error::TryRecvError::Empty)));
}

#[tokio::test]
async fn one_feed_delivers_without_status_polling_and_legacy_lifecycle_events_still_arrive() {
    let directory = tempfile::tempdir().unwrap();
    let socket = directory.path().join("service.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let (send, receive) = mpsc::channel::<Bytes>(1);
    let status_reads = Arc::new(AtomicUsize::new(0));
    let status_hits = Arc::clone(&status_reads);
    let service = Router::new().route(PATH, get(|State(receiver): State<Arc<Mutex<Option<mpsc::Receiver<Bytes>>>>>, headers: http::HeaderMap| async move {
        assert_eq!(headers[header::AUTHORIZATION], "Bearer test");
        let receiver = receiver.lock().await.take().expect("shared service feed established only once");
        let body = Body::from_stream(futures::stream::unfold(receiver, |mut receiver| async move {
            receiver.recv().await.map(|data| (Ok::<_, Infallible>(data), receiver))
        }));
        Response::builder().header(header::CONTENT_TYPE, CONTENT_TYPE).body(body).unwrap()
    })).route("/internal/gateway/status", get(move || async move {
        status_hits.fetch_add(1, Ordering::SeqCst);
        Json(json!({"sandboxes":[{"id":"lifecycle-vm","status":"Running","available_actions":[]}]}))
    })).with_state(Arc::new(Mutex::new(Some(receive))));
    let service_task = tokio::spawn(async move { axum::serve(listener, service).await.unwrap() });

    let (_, state) = crate::tests::health_app(socket.to_str().unwrap());
    let gateway = crate::gateway_router(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let gateway_task = tokio::spawn(async move {
        axum::serve(listener, gateway.into_make_service_with_connect_info::<SocketAddr>())
            .await
            .unwrap();
    });
    let relay_task = start(&state).await;
    let mut clients = Vec::new();
    for _ in 0..2 {
        let (client, _) = tokio_tungstenite::connect_async(format!("ws://{address}/events?token=test"))
            .await
            .unwrap();
        clients.push(client);
    }

    let id = uuid::Uuid::from_bytes([0x2b; 16]);
    for (event, kind) in [
        (SessionEvent::ApprovalPending(*id.as_bytes()), "approval-pending"),
        (SessionEvent::RequestDenied(*id.as_bytes()), "request-denied"),
    ] {
        send.send(Bytes::from_owner(event.encode())).await.unwrap();
        for client in &mut clients {
            let event = tokio::time::timeout(IO_TIMEOUT, client.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            assert_eq!(
                serde_json::from_str::<Value>(event.to_text().unwrap()).unwrap(),
                json!({"type":kind,"payload":{"id":id.to_string()}})
            );
        }
    }
    assert_eq!(status_reads.load(Ordering::SeqCst), 0);

    let response = crate::status::handle_status(State(state)).await;
    assert_eq!(response.status(), http::StatusCode::OK);
    for client in &mut clients {
        let event = tokio::time::timeout(IO_TIMEOUT, client.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(event.to_text().unwrap()).unwrap(),
            json!({"type":"vm-state-changed","payload":{"id":"lifecycle-vm","state":"Running","trigger":"status_poll"}})
        );
    }
    assert_eq!(status_reads.load(Ordering::SeqCst), 1);

    drop(send);
    for client in &mut clients {
        let end = tokio::time::timeout(IO_TIMEOUT, client.next()).await.unwrap();
        assert!(end.is_none_or(|message| message.is_err() || message.unwrap().is_close()));
    }
    relay_task.abort();
    gateway_task.abort();
    service_task.abort();
}
