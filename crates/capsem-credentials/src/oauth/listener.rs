//! Scoped HTTP/1 callback transport. No connection task outlives its owning future.

use std::convert::Infallible;
use std::fmt;
use std::future::ready;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bytes::Bytes;
use futures::stream::{FuturesUnordered, StreamExt};
use http_body_util::Full;
use hyper::header::{CONTENT_LENGTH, HOST, ORIGIN, TRANSFER_ENCODING, UPGRADE};
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

use super::{
    AuthorizationParameters, CallbackExchange, LoopbackRedirect, OAuthAttempt, OAuthError, OAuthPolicy, OAuthState,
};

#[derive(Debug, Clone, Copy)]
pub struct OAuthListenerPolicy {
    pub transaction: OAuthPolicy,
    pub request_timeout: Duration,
    pub max_connections: usize,
    pub max_header_bytes: usize,
    pub max_headers: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OAuthListenerError {
    OAuth(OAuthError),
    Transport,
}

impl fmt::Display for OAuthListenerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OAuth(error) => error.fmt(formatter),
            Self::Transport => formatter.write_str("OAuth callback transport unavailable"),
        }
    }
}
impl std::error::Error for OAuthListenerError {}
impl From<OAuthError> for OAuthListenerError {
    fn from(error: OAuthError) -> Self {
        Self::OAuth(error)
    }
}

/// An ephemeral listener owned by one callback transaction. Bind and wait require
/// a Tokio runtime with I/O and time enabled. Drop/cancellation closes its sockets.
#[derive(Debug)]
pub struct OAuthListener {
    listener: TcpListener,
    address: SocketAddr,
    attempt: OAuthAttempt,
    policy: OAuthListenerPolicy,
    #[cfg(test)]
    accepted: Arc<tokio::sync::Notify>,
}

impl OAuthListener {
    pub async fn bind(ip: IpAddr, path: &str, policy: OAuthListenerPolicy) -> Result<Self, OAuthListenerError> {
        let now = Instant::now();
        // Hyper's documented minimum buffer is 8192; smaller settings panic.
        if policy.max_connections == 0
            || policy.max_headers == 0
            || policy.max_header_bytes < 8192
            || policy.request_timeout.is_zero()
            || now.checked_add(policy.request_timeout).is_none()
            || policy.transaction.lifetime.is_zero()
            || now.checked_add(policy.transaction.lifetime).is_none()
            || policy.transaction.max_callback_bytes == 0
        {
            return Err(OAuthError::InvalidPolicy.into());
        }
        let listener = capsem_foundation::unix::tcp::bind_loopback(ip).map_err(|_| OAuthListenerError::Transport)?;
        let address = listener.local_addr().map_err(|_| OAuthListenerError::Transport)?;
        let redirect = LoopbackRedirect::new(address, path)?;
        let attempt = OAuthAttempt::new(redirect, policy.transaction, now)?;
        let listener = TcpListener::from_std(listener).map_err(|_| OAuthListenerError::Transport)?;
        Ok(Self {
            listener,
            address,
            attempt,
            policy,
            #[cfg(test)]
            accepted: Arc::new(tokio::sync::Notify::new()),
        })
    }

    pub fn address(&self) -> SocketAddr {
        self.address
    }

    pub fn authorization(&mut self) -> Result<AuthorizationParameters<'_>, OAuthListenerError> {
        self.attempt.authorization(Instant::now()).map_err(Into::into)
    }

    /// No task is spawned. Connections live in this future and are all dropped
    /// on cancellation. A consumed code survives a failed browser acknowledgement.
    pub async fn wait(self) -> Result<CallbackExchange, OAuthListenerError> {
        let deadline = tokio::time::Instant::from_std(self.attempt.deadline);
        let attempt = Arc::new(Mutex::new(self.attempt));
        let (sender, mut receiver) = mpsc::channel::<Outcome>(1);
        let mut connections = FuturesUnordered::new();
        loop {
            tokio::select! {
                biased;
                Some(outcome) = receiver.recv() => {
                    // Stop admitting new requests before waiting for the fixed response.
                    drop(self.listener);
                    let response_deadline = tokio::time::Instant::now() + self.policy.request_timeout;
                    while !outcome.finished.load(Ordering::Acquire) && !connections.is_empty() {
                        tokio::select! {
                            _ = tokio::time::sleep_until(response_deadline) => break,
                            _ = connections.next() => {}
                        }
                    }
                    drop(connections);
                    return outcome.result;
                }
                _ = tokio::time::sleep_until(deadline) => {
                    return Err(OAuthError::Inactive(OAuthState::Expired).into());
                }
                _ = connections.next(), if !connections.is_empty() => {}
                accepted = self.listener.accept(), if connections.len() < self.policy.max_connections => {
                    let (stream, _) = accepted.map_err(|_| OAuthListenerError::Transport)?;
                    #[cfg(test)] self.accepted.notify_one();
                    connections.push(serve(stream, self.address, self.policy, Arc::clone(&attempt), sender.clone()));
                }
            }
        }
    }
}

struct Outcome {
    result: Result<CallbackExchange, OAuthListenerError>,
    finished: Arc<AtomicBool>,
}

async fn serve(
    stream: TcpStream,
    address: SocketAddr,
    policy: OAuthListenerPolicy,
    attempt: Arc<Mutex<OAuthAttempt>>,
    sender: mpsc::Sender<Outcome>,
) {
    let finished = Arc::new(AtomicBool::new(false));
    let completion = Arc::clone(&finished);
    let service = service_fn(move |request| {
        ready(Ok::<_, Infallible>(respond(
            request, address, &attempt, &sender, &finished,
        )))
    });
    let mut builder = hyper::server::conn::http1::Builder::new();
    builder
        .keep_alive(false)
        .max_headers(policy.max_headers)
        .max_buf_size(policy.max_header_bytes)
        .header_read_timeout(None);
    // One finite budget covers partial headers and the fixed response write.
    // Transport/parser errors are never echoed or logged with callback material.
    let _ = tokio::time::timeout(
        policy.request_timeout,
        builder.serve_connection(TokioIo::new(stream), service),
    )
    .await;
    completion.store(true, Ordering::Release);
}

fn respond(
    request: Request<hyper::body::Incoming>,
    address: SocketAddr,
    attempt: &Mutex<OAuthAttempt>,
    sender: &mpsc::Sender<Outcome>,
    finished: &Arc<AtomicBool>,
) -> Response<Full<Bytes>> {
    let authority = address.to_string();
    let mut hosts = request.headers().get_all(HOST).iter();
    let host_matches =
        hosts.next().is_some_and(|host| host.as_bytes() == authority.as_bytes()) && hosts.next().is_none();
    let uri = request.uri();
    if request.method() != Method::GET
        || !host_matches
        || request.headers().contains_key(ORIGIN)
        || request.headers().contains_key(TRANSFER_ENCODING)
        || request.headers().contains_key(UPGRADE)
        || request
            .headers()
            .get(CONTENT_LENGTH)
            .is_some_and(|size| size.as_bytes() != b"0")
        || uri.scheme().is_some()
        || uri.authority().is_some()
        || !uri.path().starts_with('/')
    {
        return response(StatusCode::BAD_REQUEST, "Invalid authorization callback.");
    }
    // The origin is the real bound socket. An untrusted Host never forms the URI.
    let callback = format!("http://{address}{uri}");
    let result = attempt
        .lock()
        .map_err(|_| OAuthListenerError::Transport)
        .and_then(|mut owner| owner.accept(&callback, Instant::now()).map_err(Into::into));
    let (status, message) = match &result {
        Ok(_) => (StatusCode::OK, "Authorization response received. Return to Capsem."),
        Err(OAuthListenerError::OAuth(OAuthError::Denied)) => {
            (StatusCode::OK, "Authorization was declined. Return to Capsem.")
        }
        Err(OAuthListenerError::OAuth(OAuthError::ProviderRejected)) => (
            StatusCode::BAD_REQUEST,
            "Authorization could not be completed. Return to Capsem.",
        ),
        Err(OAuthListenerError::Transport | OAuthListenerError::OAuth(OAuthError::Inactive(OAuthState::Expired))) => (
            StatusCode::BAD_REQUEST,
            "Authorization could not be completed. Return to Capsem.",
        ),
        Err(_) => return response(StatusCode::BAD_REQUEST, "Invalid authorization callback."),
    };
    // A matched terminal result is already owned even if the browser disconnects.
    let _ = sender.try_send(Outcome {
        result,
        finished: Arc::clone(finished),
    });
    response(status, message)
}

fn response(status: StatusCode, message: &'static str) -> Response<Full<Bytes>> {
    let mut response = Response::new(Full::new(Bytes::from_static(message.as_bytes())));
    *response.status_mut() = status;
    for (name, value) in [
        ("content-type", "text/plain; charset=utf-8"),
        ("cache-control", "no-store"),
        ("referrer-policy", "no-referrer"),
        ("content-security-policy", "default-src 'none'"),
        ("x-content-type-options", "nosniff"),
        ("connection", "close"),
    ] {
        response
            .headers_mut()
            .insert(name, hyper::header::HeaderValue::from_static(value));
    }
    response
}

#[cfg(test)]
mod tests;
