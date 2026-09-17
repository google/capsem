//! HTTP preview transport inside the confined descriptor worker.
//!
//! The gateway authenticates a browser connection without consuming it. This
//! module is the only component that parses workload HTTP: it streams bodies,
//! strips Capsem control credentials, preserves workload-owned cookies, and
//! relays upgrades.

use capsem_proto::PreviewAdmissionKind;
use http_body_util::{combinators::BoxBody, BodyExt, Empty};
use hyper::header::{AUTHORIZATION, COOKIE, PROXY_AUTHORIZATION, SET_COOKIE};
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use std::convert::Infallible;
use std::future::Future;
use std::os::fd::AsFd;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::net::UnixStream;
use tokio::sync::{Mutex, Notify};
use tokio_util::sync::CancellationToken;

use capsem_proto::PREVIEW_COOKIE;
const DOWNSTREAM_DRAIN: std::time::Duration = std::time::Duration::from_secs(2);

struct Upgrades {
    active: AtomicUsize,
    drained: Notify,
}

impl Upgrades {
    fn spawn(self: &Arc<Self>, downstream: hyper::upgrade::OnUpgrade, upstream: hyper::upgrade::OnUpgrade) {
        self.active.fetch_add(1, Ordering::Relaxed);
        let tracked = Arc::clone(self);
        tokio::spawn(async move {
            if let Ok((downstream, upstream)) = tokio::try_join!(downstream, upstream) {
                let mut downstream = TokioIo::new(downstream);
                let mut upstream = TokioIo::new(upstream);
                if let Err(error) = tokio::io::copy_bidirectional(&mut downstream, &mut upstream).await {
                    tracing::debug!(%error, "preview WebSocket relay ended");
                }
            }
            if tracked.active.fetch_sub(1, Ordering::AcqRel) == 1 {
                tracked.drained.notify_waiters();
            }
        });
    }

    async fn wait(&self) {
        while self.active.load(Ordering::Acquire) != 0 {
            self.drained.notified().await;
        }
    }
}

pub(super) async fn relay(
    source: &mut UnixStream,
    destination: &mut UnixStream,
    admission: PreviewAdmissionKind,
    stop: impl Future<Output = ()>,
) -> capsem_foundation::unix::router_stream::Outcome {
    use capsem_foundation::unix::router_stream::{self, Framing, Framings, Limits};

    let browser = match capsem_foundation::unix::fd::duplicate(source.as_fd()).and_then(super::adopt) {
        Ok(browser) => browser,
        Err(error) => {
            return router_stream::Outcome {
                from_source: 0,
                to_source: 0,
                reason: router_stream::CloseReason::Io,
                error: Some(error),
            };
        }
    };
    let (upstream, mut framed) = tokio::io::duplex(router_stream::SOCKET_BUFFER_SIZE);
    let local_stop = CancellationToken::new();
    let finish = local_stop.clone();
    let copy = router_stream::copy_until(
        &mut framed,
        destination,
        Framings {
            source: Framing::Raw,
            destination: Framing::Framed,
        },
        Limits::default(),
        async move {
            tokio::select! {
                _ = stop => {}
                _ = finish.cancelled() => {}
            }
        },
    );
    tokio::pin!(copy);

    let handshake = hyper::client::conn::http1::handshake(TokioIo::new(upstream));
    tokio::pin!(handshake);
    let (sender, connection) = tokio::select! {
        result = &mut handshake => match result {
            Ok(parts) => parts,
            Err(error) => {
                local_stop.cancel();
                tracing::debug!(%error, "preview guest HTTP handshake failed");
                return copy.await;
            }
        },
        outcome = &mut copy => return outcome,
    };
    let client = tokio::spawn(async move { connection.with_upgrades().await });
    let sender = Arc::new(Mutex::new(sender));
    let upgrades = Arc::new(Upgrades {
        active: AtomicUsize::new(0),
        drained: Notify::new(),
    });
    let service_upgrades = Arc::clone(&upgrades);
    let service =
        service_fn(move |request| forward(request, admission, Arc::clone(&sender), Arc::clone(&service_upgrades)));
    let server = hyper::server::conn::http1::Builder::new()
        .serve_connection(TokioIo::new(browser), service)
        .with_upgrades();
    tokio::pin!(server);
    let server_result = tokio::select! {
        result = &mut server => Some(result),
        outcome = &mut copy => {
            // The guest's framed EOF completes `copy` as soon as Hyper has
            // received the response body. Give the downstream server time to
            // flush its final chunk before closing the browser descriptor.
            let _ = tokio::time::timeout(DOWNSTREAM_DRAIN, &mut server).await;
            client.abort();
            return outcome;
        }
    };
    if let Some(Err(error)) = server_result {
        tracing::debug!(%error, "preview browser HTTP connection ended");
    }
    let wait_upgrades = upgrades.wait();
    tokio::pin!(wait_upgrades);
    tokio::select! {
        // Preserve an already-observed guest EOF instead of relabeling the
        // completed transfer as cooperative cancellation.
        biased;
        outcome = &mut copy => {
            client.abort();
            return outcome;
        }
        () = &mut wait_upgrades => local_stop.cancel(),
    }
    let outcome = copy.await;
    client.abort();
    outcome
}

async fn forward(
    mut request: Request<hyper::body::Incoming>,
    admission: PreviewAdmissionKind,
    sender: Arc<Mutex<hyper::client::conn::http1::SendRequest<hyper::body::Incoming>>>,
    upgrades: Arc<Upgrades>,
) -> Result<Response<BoxBody<bytes::Bytes, hyper::Error>>, Infallible> {
    let websocket = is_websocket(&request);
    // The gateway's policy admitted this connection from its first request
    // head. Keep-alive must not let a later request take the other shape.
    if websocket != (admission == PreviewAdmissionKind::WebsocketUpgrade) {
        tracing::debug!(
            ?admission,
            websocket,
            "preview request outside its admitted kind refused"
        );
        return Ok(Response::builder()
            .status(StatusCode::FORBIDDEN)
            .header(hyper::header::CONNECTION, "close")
            .body(empty_body())
            .expect("static response"));
    }
    let downstream = websocket.then(|| hyper::upgrade::on(&mut request));
    strip_control_headers(request.headers_mut());
    let response = sender.lock().await.send_request(request).await;
    let mut response = match response {
        Ok(response) => response,
        Err(error) => {
            tracing::debug!(%error, "preview guest HTTP request failed");
            return Ok(Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .body(empty_body())
                .expect("static response"));
        }
    };
    strip_control_set_cookies(response.headers_mut());
    if response.status() == StatusCode::SWITCHING_PROTOCOLS {
        if let Some(downstream) = downstream {
            upgrades.spawn(downstream, hyper::upgrade::on(&mut response));
        } else {
            return Ok(Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .body(empty_body())
                .expect("static response"));
        }
    }
    let (parts, body) = response.into_parts();
    Ok(Response::from_parts(parts, body.boxed()))
}

fn empty_body() -> BoxBody<bytes::Bytes, hyper::Error> {
    Empty::<bytes::Bytes>::new().map_err(|never| match never {}).boxed()
}

/// Cookie values are bytes: hyper accepts 0x80-0xFF, which a workload may well
/// have set, while `HeaderValue::to_str` refuses them. Filtering on `to_str`
/// dropped the whole header over one such byte and logged the user out of the
/// workload, so the split and rebuild work on bytes.
fn strip_control_headers(headers: &mut hyper::HeaderMap) {
    headers.remove(AUTHORIZATION);
    headers.remove(PROXY_AUTHORIZATION);
    let reserved = format!("{PREVIEW_COOKIE}=").into_bytes();
    let mut retained: Vec<u8> = Vec::new();
    for pair in headers
        .get_all(COOKIE)
        .iter()
        .flat_map(|value| value.as_bytes().split(|byte| *byte == b';'))
        .map(trim_ascii)
        .filter(|pair| !pair.starts_with(&reserved))
        .filter(|pair| !pair.is_empty())
    {
        if !retained.is_empty() {
            retained.extend_from_slice(b"; ");
        }
        retained.extend_from_slice(pair);
    }
    headers.remove(COOKIE);
    if !retained.is_empty() {
        if let Ok(value) = hyper::header::HeaderValue::from_bytes(&retained) {
            headers.insert(COOKIE, value);
        }
    }
}

fn trim_ascii(value: &[u8]) -> &[u8] {
    let start = value.iter().position(|byte| !byte.is_ascii_whitespace());
    let end = value.iter().rposition(|byte| !byte.is_ascii_whitespace());
    match (start, end) {
        (Some(start), Some(end)) => &value[start..=end],
        _ => &[],
    }
}

fn strip_control_set_cookies(headers: &mut hyper::HeaderMap) {
    let retained = headers
        .get_all(SET_COOKIE)
        .iter()
        .filter(|value| {
            let first = value.as_bytes().split(|byte| *byte == b';').next().unwrap_or_default();
            let name = first.split(|byte| *byte == b'=').next().unwrap_or_default();
            trim_ascii(name) != PREVIEW_COOKIE.as_bytes()
        })
        .cloned()
        .collect::<Vec<_>>();
    headers.remove(SET_COOKIE);
    for value in retained {
        headers.append(SET_COOKIE, value);
    }
}

fn is_websocket<B>(request: &Request<B>) -> bool {
    request
        .headers()
        .get(hyper::header::UPGRADE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.eq_ignore_ascii_case("websocket"))
}

#[cfg(test)]
mod tests;
