use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::{body::Body, extract::State, http::Request, response::IntoResponse, Json};
use serde_json::{json, Value};
use tokio::sync::{mpsc, Notify};

use crate::{models::ManagedSessionOutcome, CreateOptions, Error, Hypervisor};

const GENERATION: &str = "22222222-2222-4222-8222-222222222222";

#[derive(Default)]
struct Fixture {
    managed: Mutex<Value>,
    requests: Mutex<Vec<(String, Value)>>,
    held_create: bool,
    failed_close: std::sync::atomic::AtomicBool,
    bad_generation: bool,
    short_expiry: bool,
    lost_create_reply: bool,
    first_claim_missing: std::sync::atomic::AtomicBool,
    claim_state: Option<&'static str>,
    close_release: Notify,
    events: Mutex<Option<mpsc::UnboundedSender<String>>>,
}

async fn respond(State(state): State<Arc<Fixture>>, request: Request<Body>) -> axum::response::Response {
    let path = request.uri().to_string();
    let body: Value = serde_json::from_slice(&axum::body::to_bytes(request.into_body(), 65536).await.unwrap()).unwrap();
    state.requests.lock().unwrap().push((path.clone(), body.clone()));
    if path == "/vms/create" {
        *state.managed.lock().unwrap() = body["managed"].clone();
    }
    state
        .events
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .send(path.clone())
        .unwrap();
    let managed = state.managed.lock().unwrap().clone();
    let mut ownership = json!({"request_id":managed["request_id"],"generation":GENERATION,
        "expires_at":"2099-01-01T00:00:00Z","lease_seconds":3,"renew_after_seconds":1});
    if state.short_expiry {
        ownership["expires_at"] =
            json!(humantime::format_rfc3339(std::time::SystemTime::now() + Duration::from_millis(200)).to_string());
    }
    if path == "/vms/create" {
        if state.held_create {
            std::future::pending::<()>().await;
        }
        if state.lost_create_reply {
            return (
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                "reply lost after create acceptance",
            )
                .into_response();
        }
        return Json(json!({"id":"vm-original","name":"vm-original","status":"Running","available_actions":[],"persistent":false,"ownership":ownership})).into_response();
    }
    assert_eq!(body["ownership_token"], managed["ownership_token"]);
    assert!(!path.contains(managed["ownership_token"].as_str().unwrap()));
    if path.ends_with("/claim") {
        if state
            .first_claim_missing
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            return (axum::http::StatusCode::NOT_FOUND, "reservation not published").into_response();
        }
        let mut ownership = ownership;
        if state.bad_generation {
            ownership["generation"] = json!("33333333-3333-4333-8333-333333333333");
        }
        let status = state
            .claim_state
            .unwrap_or(if state.held_create { "pending" } else { "active" });
        return Json(json!({"ownership":ownership,"state":status})).into_response();
    }
    assert!(path.ends_with("/close"));
    if state.failed_close.swap(false, std::sync::atomic::Ordering::SeqCst) {
        return (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error":"cleanup failed"})),
        )
            .into_response();
    }
    state.close_release.notified().await;
    state
        .events
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .send("/closed".into())
        .unwrap();
    Json(
        json!({"request_id":managed["request_id"],"generation":GENERATION,"state":"closed",
        "session_id":"vm-original","evidence_retained":false}),
    )
    .into_response()
}

#[tokio::test]
async fn ownership_expiry_cancels_body_before_renewal_interval() {
    let mut gateway = Gateway::new(Fixture {
        short_expiry: true,
        ..Default::default()
    })
    .await;
    let hv = gateway.hv.clone();
    let scope = tokio::spawn(async move {
        hv.with_managed(CreateOptions::default(), |_, _| {
            std::future::pending::<crate::Result<()>>()
        })
        .await
    });
    gateway.event("/create").await;
    let path = tokio::time::timeout(Duration::from_millis(750), gateway.events.recv())
        .await
        .expect("expiry must end ownership before renewal interval")
        .unwrap();
    assert!(path.ends_with("/close"));
    assert!(!scope.is_finished());
    gateway.state.close_release.notify_one();
    assert!(scope.await.unwrap().is_err());
}

#[tokio::test]
async fn uncertain_create_reply_never_enters_body_or_replays_mutation() {
    let gateway = Gateway::new(Fixture {
        lost_create_reply: true,
        ..Default::default()
    })
    .await;
    let entered = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let seen = entered.clone();
    gateway.state.close_release.notify_one();
    let result = gateway
        .hv
        .with_managed(CreateOptions::default(), move |_, _| async move {
            seen.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        })
        .await;
    assert!(matches!(result, Err(Error::Http { status: 503, .. })));
    assert!(!entered.load(std::sync::atomic::Ordering::SeqCst));
    let requests = gateway.state.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests.last().unwrap().1["outcome"], "failed");
    drop(requests);
}

#[tokio::test]
async fn pending_reservation_retries_only_unknown_claim_and_still_joins_close() {
    let mut gateway = Gateway::new(Fixture {
        held_create: true,
        first_claim_missing: true.into(),
        ..Default::default()
    })
    .await;
    let hv = gateway.hv.clone();
    let scope = tokio::spawn(async move {
        hv.with_managed(CreateOptions::default(), |_, _| {
            std::future::pending::<crate::Result<()>>()
        })
        .await
    });
    gateway.event("/create").await;
    gateway.event("/claim").await;
    gateway.event("/claim").await;
    scope.abort();
    assert!(scope.await.unwrap_err().is_cancelled());
    gateway.event("/close").await;
    gateway.state.close_release.notify_one();
    gateway.event("/closed").await;
    assert_eq!(
        gateway
            .state
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|(p, _)| p == "/vms/create")
            .count(),
        1
    );
}

#[tokio::test]
async fn terminal_claims_stop_callback_and_named_inputs_never_mutate() {
    for state in ["closed", "failed"] {
        let gateway = Gateway::new(Fixture {
            claim_state: Some(state),
            ..Default::default()
        })
        .await;
        gateway.state.close_release.notify_one();
        let result = gateway
            .hv
            .with_managed(CreateOptions::default(), |_, _| {
                std::future::pending::<crate::Result<()>>()
            })
            .await;
        assert!(matches!(result, Err(Error::Managed("ownership is no longer live"))));
    }
    let gateway = Gateway::new(Fixture::default()).await;
    for name in ["work", ""] {
        assert!(matches!(
            gateway
                .hv
                .with_managed(
                    CreateOptions {
                        name: Some(name.into()),
                        ..Default::default()
                    },
                    |_, _| async { Ok(()) }
                )
                .await,
            Err(Error::InvalidInput(_))
        ));
    }
    assert!(gateway.state.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn callback_cancellation_and_panic_both_await_close() {
    let gateway = Gateway::new(Fixture::default()).await;
    gateway.state.close_release.notify_one();
    let result = gateway
        .hv
        .with_managed(CreateOptions::default(), |_, cancel| async move {
            cancel.cancel();
            std::future::pending::<crate::Result<()>>().await
        })
        .await;
    assert!(matches!(result, Err(Error::Cancelled)));
    assert_eq!(
        gateway.state.requests.lock().unwrap().last().unwrap().1["outcome"],
        "cancelled"
    );
    let gateway = Gateway::new(Fixture::default()).await;
    gateway.state.close_release.notify_one();
    let result = gateway
        .hv
        .with_managed(
            CreateOptions::default(),
            |_, _| -> std::future::Ready<crate::Result<()>> {
                panic!("callback invocation panic");
            },
        )
        .await;
    assert!(matches!(result, Err(Error::Managed("managed callback panicked"))));
    assert_eq!(
        gateway.state.requests.lock().unwrap().last().unwrap().1["outcome"],
        "failed"
    );
}

struct Gateway {
    hv: Hypervisor,
    state: Arc<Fixture>,
    events: mpsc::UnboundedReceiver<String>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Gateway {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Gateway {
    async fn new(state: Fixture) -> Self {
        let state = Arc::new(state);
        let (sent, events) = mpsc::unbounded_channel();
        *state.events.lock().unwrap() = Some(sent);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let hv = Hypervisor::new(&format!("http://{}", listener.local_addr().unwrap()), "gateway-secret").unwrap();
        let app = axum::Router::new().fallback(respond).with_state(state.clone());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            hv,
            state,
            events,
            task,
        }
    }
    async fn event(&mut self, suffix: &str) {
        let path = tokio::time::timeout(Duration::from_secs(4), self.events.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(path.ends_with(suffix), "{path}");
    }
}

#[tokio::test]
async fn scope_renews_and_joins_close_without_replaying_create() {
    let mut gateway = Gateway::new(Fixture::default()).await;
    let hv = gateway.hv.clone();
    let scope = tokio::spawn(async move {
        hv.with_managed(
            CreateOptions {
                labels: Some([("suite".to_string(), "eval".to_string())].into()),
                ..Default::default()
            },
            |vm, _cancel| async move {
                assert_eq!(vm.id(), Some("vm-original"));
                tokio::time::sleep(Duration::from_millis(1100)).await;
                Ok(17)
            },
        )
        .await
    });
    gateway.event("/create").await;
    gateway.event("/claim").await;
    gateway.event("/close").await;
    assert!(!scope.is_finished());
    gateway.state.close_release.notify_one();
    assert_eq!(scope.await.unwrap().unwrap(), 17);
    let requests = gateway.state.requests.lock().unwrap();
    assert_eq!(requests.iter().filter(|(p, _)| p == "/vms/create").count(), 1);
    assert_eq!(requests[0].1["labels"], json!({"suite": "eval"}));
    assert_eq!(requests.last().unwrap().1["outcome"], "completed");
    drop(requests);
}

#[tokio::test]
async fn dropped_caller_during_create_leaves_owned_cleanup_running() {
    let mut gateway = Gateway::new(Fixture {
        held_create: true,
        ..Default::default()
    })
    .await;
    let hv = gateway.hv.clone();
    let scope = tokio::spawn(async move {
        hv.with_managed(CreateOptions::default(), |_, _| async {
            Err::<(), _>(Error::Managed("cancelled create entered body"))
        })
        .await
    });
    gateway.event("/create").await;
    scope.abort();
    assert!(scope.await.unwrap_err().is_cancelled());
    gateway.event("/close").await;
    assert_eq!(
        gateway.state.requests.lock().unwrap().last().unwrap().1["outcome"],
        "cancelled"
    );
    gateway.state.close_release.notify_one();
    gateway.event("/closed").await;
}

#[tokio::test]
async fn renewal_identity_failure_cancels_body_and_joins_cleanup() {
    let mut gateway = Gateway::new(Fixture {
        bad_generation: true,
        ..Default::default()
    })
    .await;
    let hv = gateway.hv.clone();
    let (cancelled, seen) = tokio::sync::oneshot::channel();
    let scope = tokio::spawn(async move {
        hv.with_managed(CreateOptions::default(), |_, cancel| async move {
            let watcher = tokio::spawn(async move {
                cancel.cancelled().await;
                let _ = cancelled.send(());
            });
            let _ = watcher.await;
            Ok(())
        })
        .await
    });
    gateway.event("/create").await;
    gateway.event("/claim").await;
    gateway.event("/close").await;
    tokio::time::timeout(Duration::from_secs(1), seen)
        .await
        .unwrap()
        .unwrap();
    assert!(!scope.is_finished());
    gateway.state.close_release.notify_one();
    assert!(scope.await.unwrap().is_err());
}

#[tokio::test]
async fn failed_cleanup_exposes_only_explicit_retry_handle() {
    let gateway = Gateway::new(Fixture {
        failed_close: true.into(),
        ..Default::default()
    })
    .await;
    let error = gateway
        .hv
        .with_managed(CreateOptions::default(), |_, _| async { Ok(()) })
        .await
        .unwrap_err();
    let Error::ManagedCleanup { mut session, .. } = error else {
        panic!("failed close must retain recovery authority");
    };
    let secret = gateway.state.managed.lock().unwrap()["ownership_token"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(!format!("{session:?}").contains(&secret));
    assert_eq!(
        gateway
            .state
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|(p, _)| p.ends_with("/close"))
            .count(),
        1
    );
    gateway.state.close_release.notify_one();
    session.close(ManagedSessionOutcome::Failed).await.unwrap();
    session.close(ManagedSessionOutcome::Failed).await.unwrap();
    assert_eq!(
        gateway
            .state
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|(p, _)| p.ends_with("/close"))
            .count(),
        2
    );
}
