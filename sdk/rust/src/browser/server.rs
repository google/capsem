use std::convert::Infallible;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::Full;
use hyper::server::conn::http1;
use hyper_util::rt::{TokioIo, TokioTimer};
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use super::{preview::Preview, CallbackPlan};
use crate::{Error, Result};

struct State {
    plan: CallbackPlan,
    preview: Preview,
    used: AtomicBool,
}

pub(super) async fn run(
    listeners: Vec<TcpListener>,
    plan: CallbackPlan,
    preview: Preview,
    cancel: CancellationToken,
    lifetime: Duration,
) -> Result<()> {
    let state = Arc::new(State {
        plan,
        preview,
        used: AtomicBool::new(false),
    });
    let mut families = JoinSet::new();
    // At most two workers, one per reserved loopback family. Each owns one
    // connection at a time; no task-per-connection or idle polling allocation.
    for listener in listeners {
        let state = Arc::clone(&state);
        families.spawn(async move { serve(listener, state).await });
    }
    let outcome = tokio::select! {
        _ = cancel.cancelled() => Ok(()),
        _ = tokio::time::sleep(lifetime) => Ok(()),
        result = families.join_next() => result
            .ok_or(Error::InvalidResponse("callback listener ended"))?
            .map_err(|_| Error::InvalidResponse("callback worker ended"))?,
    };
    families.shutdown().await;
    outcome
}

async fn serve(listener: TcpListener, state: Arc<State>) -> Result<()> {
    loop {
        let (stream, _) = listener
            .accept()
            .await
            .map_err(|_| Error::InvalidResponse("callback listener failed"))?;
        let request_state = Arc::clone(&state);
        let owned = Arc::new(AtomicBool::new(false));
        let request_owned = Arc::clone(&owned);
        let service = hyper::service::service_fn(move |request| {
            let state = Arc::clone(&request_state);
            let owned = Arc::clone(&request_owned);
            async move {
                let response = if !state.plan.admits(&request) {
                    refusal(403, "Callback does not match this sign-in")
                } else if state.used.swap(true, Ordering::AcqRel) {
                    refusal(410, "Sign-in attempt has ended")
                } else {
                    owned.store(true, Ordering::Release);
                    state
                        .preview
                        .forward(request.uri())
                        .await
                        .unwrap_or_else(|_| refusal(502, "Could not return sign-in to the application"))
                };
                Ok::<_, Infallible>(response)
            }
        });
        let mut builder = http1::Builder::new();
        builder
            .timer(TokioTimer::new())
            .header_read_timeout(Duration::from_secs(5))
            .max_buf_size(16384)
            .keep_alive(false);
        let _ = tokio::time::timeout(
            Duration::from_secs(25),
            builder.serve_connection(TokioIo::new(stream), service),
        )
        .await;
        // The other address family must not abort this family's in-flight
        // accepted response merely because it sees the shared replay bit.
        if owned.load(Ordering::Acquire) {
            return Ok(());
        }
    }
}

fn refusal(status: u16, message: &'static str) -> http::Response<Full<Bytes>> {
    http::Response::builder()
        .status(status)
        .header("Cache-Control", "no-store")
        .header("Connection", "close")
        .header("Content-Type", "text/plain; charset=utf-8")
        .body(Full::new(Bytes::from_static(message.as_bytes())))
        .expect("static response")
}
