//! Shared Axum adapter; frames pass through unchanged and are never inspected.

use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Instant;

use axum::body::Body as AxumBody;
use axum::extract::{MatchedPath, Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::Response;
use http_body::{Body, Frame, SizeHint};
use metrics::SharedString;
use tracing::Instrument;

use super::{BODY_LIFETIME_MS, REQUESTS_TOTAL, RESPONSES_TOTAL, RESPONSE_READY_MS};

#[derive(Clone, Copy, Debug)]
pub enum Component {
    Gateway,
    Service,
}

impl Component {
    fn label(self) -> &'static str {
        match self {
            Self::Gateway => "gateway",
            Self::Service => "service",
        }
    }
}

fn method_label(method: &str) -> &'static str {
    match method {
        "GET" => "GET",
        "HEAD" => "HEAD",
        "POST" => "POST",
        "PUT" => "PUT",
        "PATCH" => "PATCH",
        "DELETE" => "DELETE",
        "OPTIONS" => "OPTIONS",
        "CONNECT" => "CONNECT",
        "TRACE" => "TRACE",
        _ => "OTHER",
    }
}

/// Uses only Axum's registered route template. Never fall back to the URI.
pub async fn observe(State(component): State<Component>, request: Request, next: Next) -> Response {
    let method = method_label(request.method().as_str());
    let route = SharedString::const_str(super::operations::normalized_route(
        request.extensions().get::<MatchedPath>().map(MatchedPath::as_str),
    ));
    let span = tracing::debug_span!(
        "http.request",
        component = component.label(),
        method,
        route = route.as_ref()
    );
    let observation = RequestObservation::begin(component, method, route);
    let response = next.run(request).instrument(span.clone()).await;
    span.in_scope(|| tracing::debug!(status = response.status().as_u16(), "response ready"));
    let (parts, body) = response.into_parts();
    let status = parts.status;
    Response::from_parts(parts, AxumBody::new(observation.ready(status, body)))
}

struct RequestObservation {
    component: &'static str,
    method: &'static str,
    route: SharedString,
    started: Instant,
    status: SharedString,
    ready: bool,
    finished: bool,
}

impl RequestObservation {
    fn begin(component: Component, method: &'static str, route: SharedString) -> Self {
        metrics::counter!(REQUESTS_TOTAL, "component" => component.label(), "method" => method, "route" => route.clone()).increment(1);
        Self {
            component: component.label(),
            method,
            route,
            started: Instant::now(),
            status: "none".into(),
            ready: false,
            finished: false,
        }
    }

    fn ready<B: Body + Unpin>(mut self, status: StatusCode, body: B) -> ObservedBody<B> {
        self.status = status.as_str().to_owned().into();
        metrics::histogram!(RESPONSE_READY_MS, "component" => self.component, "method" => self.method,
            "route" => self.route.clone(), "status" => self.status.clone())
        .record(self.started.elapsed().as_secs_f64() * 1000.0);
        self.ready = true;
        self.started = Instant::now();
        if status == StatusCode::SWITCHING_PROTOCOLS {
            self.finish("upgraded");
        } else if body.is_end_stream() {
            self.finish("eof");
        }
        ObservedBody {
            inner: body,
            observation: self,
        }
    }

    fn finish(&mut self, completion: &'static str) {
        if self.finished {
            return;
        }
        self.finished = true;
        metrics::counter!(RESPONSES_TOTAL, "component" => self.component, "method" => self.method,
            "route" => self.route.clone(), "status" => self.status.clone(), "completion" => completion)
        .increment(1);
        if self.ready && completion != "upgraded" {
            metrics::histogram!(BODY_LIFETIME_MS, "component" => self.component, "method" => self.method,
                "route" => self.route.clone(), "status" => self.status.clone(), "completion" => completion)
            .record(self.started.elapsed().as_secs_f64() * 1000.0);
        }
    }
}

impl Drop for RequestObservation {
    fn drop(&mut self) {
        self.finish(if self.ready { "dropped" } else { "cancelled" });
    }
}

struct ObservedBody<B> {
    inner: B,
    observation: RequestObservation,
}

impl<B: Body + Unpin> Body for ObservedBody<B> {
    type Data = B::Data;
    type Error = B::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<B::Data>, B::Error>>> {
        let result = Pin::new(&mut self.inner).poll_frame(context);
        match &result {
            Poll::Ready(None) => self.observation.finish("eof"),
            Poll::Ready(Some(Err(_))) => self.observation.finish("body_error"),
            Poll::Ready(Some(Ok(frame))) if frame.is_trailers() || self.inner.is_end_stream() => {
                self.observation.finish("eof")
            }
            _ => {}
        }
        result
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

#[cfg(test)]
mod tests;
