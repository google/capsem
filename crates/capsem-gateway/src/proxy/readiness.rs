//! Wait planning reads the service's verified selection before a long mutation.
use super::*;
use axum::body::Bytes;
use axum::extract::{FromRequestParts, RawPathParams};
use capsem_api::{
    environments::{EnvironmentReadinessResponse, EnvironmentRevisionReference},
    ProvisionRequest, VmLifecycleSelection, VmReadinessResponse, VmStartRequest,
};

const METADATA_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_METADATA_BYTES: usize = 4096;

pub(super) enum Mutation {
    Create,
    Start(String),
}

impl Mutation {
    pub(super) async fn for_request(parts: &mut http::request::Parts) -> Option<Self> {
        if parts.method != Method::POST {
            return None;
        }
        let path = parts.uri.path();
        if path == "/vms/create" {
            return Some(Self::Create);
        }
        let (id, operation) = path.strip_prefix("/vms/")?.rsplit_once('/')?;
        if !matches!(operation, "start" | "resume") {
            return None;
        }
        let id = if id.contains('%') {
            let parameters = RawPathParams::from_request_parts(parts, &()).await.ok()?;
            parameters
                .iter()
                .find_map(|(key, value)| (key == "id").then_some(value))?
                .to_owned()
        } else {
            id.to_owned()
        };
        capsem_api::validate_name("VM", &id).ok()?;
        Some(Self::Start(id))
    }

    pub(super) async fn plan(
        self,
        state: &AppState,
        headers: &HeaderMap,
        body: Bytes,
        fallback: Duration,
    ) -> Result<(Duration, Bytes), Response> {
        match self {
            Self::Create => plan_create(state, headers, body, fallback).await,
            Self::Start(id) => plan_start(state, headers, body, fallback, &id).await,
        }
    }
}

async fn plan_start(
    state: &AppState,
    headers: &HeaderMap,
    body: Bytes,
    fallback: Duration,
    id: &str,
) -> Result<(Duration, Bytes), Response> {
    let mut request = if body.is_empty() {
        VmStartRequest::default()
    } else {
        let Some(request) = request_object::<VmStartRequest>(&body) else {
            return Ok((fallback, body));
        };
        request
    };
    if request.validate(id).is_err() {
        return Ok((fallback, body));
    }
    let readiness = lifecycle_readiness(state, headers, id, "start", request.expected.is_some()).await?;
    readiness
        .selection
        .validate_target(id)
        .map_err(|_| lifecycle_unavailable())?;
    let deadline = if readiness.selection.environment.is_some() {
        preparation_deadline(readiness.startup_timeout_seconds)
    } else {
        fallback
    };
    let body = if bind_selection(&mut request.expected, readiness.selection)? {
        encode(&request)?
    } else {
        body
    };
    Ok((deadline, body))
}

async fn plan_create(
    state: &AppState,
    headers: &HeaderMap,
    body: Bytes,
    fallback: Duration,
) -> Result<(Duration, Bytes), Response> {
    let Some(mut request) = request_object::<ProvisionRequest>(&body) else {
        return Ok((fallback, body));
    };
    // Invalid payloads still reach the authoritative service validator.
    if request.validate_environment().is_err() {
        return Ok((fallback, body));
    }

    let mut inherited_seconds = None;
    let mut bound = false;
    if let Some(source) = &request.from {
        if capsem_api::validate_name("VM source", source).is_err() {
            return Ok((fallback, body));
        }
        let readiness = lifecycle_readiness(state, headers, source, "clone", request.expected_source.is_some()).await?;
        readiness.selection.validate().map_err(|_| lifecycle_unavailable())?;
        if readiness.selection.environment.is_some() && request.container.is_none() {
            inherited_seconds = Some(readiness.startup_timeout_seconds);
        }
        bound = bind_selection(&mut request.expected_source, readiness.selection)?;
    }

    let deadline = if request.attach {
        fallback
    } else if let Some(environment) = &request.environment {
        preparation_deadline(environment_timeout(state, headers, environment).await?)
    } else {
        inherited_seconds.map_or(fallback, preparation_deadline)
    };
    let body = if bound { encode(&request)? } else { body };
    Ok((deadline, body))
}

fn request_object<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Option<T> {
    if !bytes.trim_ascii_start().starts_with(b"{") {
        return None;
    }
    serde_json::from_slice(bytes).ok()
}

/// Older clients can omit the expectation. The service consumes the exact
/// selection atomically in either case, even if it changes after this read.
fn bind_selection(
    expected: &mut Option<VmLifecycleSelection>,
    selection: VmLifecycleSelection,
) -> Result<bool, Response> {
    match expected {
        Some(previous) if *previous != selection => Err(changed()),
        Some(_) => Ok(false),
        None => {
            *expected = Some(selection);
            Ok(true)
        }
    }
}

fn encode(value: &impl serde::Serialize) -> Result<Bytes, Response> {
    let body = serde_json::to_vec(value).map_err(|_| lifecycle_unavailable())?;
    if body.len() > MAX_BODY_SIZE {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            axum::Json(serde_json::json!({"error":"request body too large"})),
        )
            .into_response());
    }
    Ok(body.into())
}

async fn lifecycle_readiness(
    state: &AppState,
    headers: &HeaderMap,
    id: &str,
    intent: &str,
    expected: bool,
) -> Result<VmReadinessResponse, Response> {
    let uri = format!("http://localhost/vms/{id}/readiness?intent={intent}");
    lookup(state, headers, &uri, lifecycle_unavailable)
        .await
        .map_err(|error| {
            if expected && error.status() == StatusCode::NOT_FOUND {
                changed()
            } else {
                error
            }
        })
}

async fn environment_timeout(
    state: &AppState,
    headers: &HeaderMap,
    environment: &EnvironmentRevisionReference,
) -> Result<u32, Response> {
    let uri = format!(
        "http://localhost/environments/{}/revisions/{}/readiness",
        environment.environment_id, environment.revision_id
    );
    let response: EnvironmentReadinessResponse = lookup(state, headers, &uri, environment_unavailable).await?;
    if response.environment != *environment {
        return Err(environment_unavailable());
    }
    Ok(response.startup_timeout_seconds)
}

async fn lookup<T: serde::de::DeserializeOwned>(
    state: &AppState,
    headers: &HeaderMap,
    uri: &str,
    unavailable: fn() -> Response,
) -> Result<T, Response> {
    let lookup = async {
        let mut request = hyper::Request::builder().method(Method::GET).uri(uri);
        if let Some(authorization) = headers.get(http::header::AUTHORIZATION) {
            request = request.header(http::header::AUTHORIZATION, authorization.clone());
        }
        let response = state
            .service_client
            .request(request.body(axum::body::Body::empty()).map_err(|_| ())?)
            .await
            .map_err(|_| ())?;
        let status = response.status();
        let body = Limited::new(response.into_body(), MAX_METADATA_BYTES)
            .collect()
            .await
            .map_err(|_| ())?
            .to_bytes();
        Ok::<_, ()>((status, body))
    };
    let (status, bytes) = match tokio::time::timeout(METADATA_TIMEOUT, lookup).await {
        Ok(Ok(response)) => response,
        _ => return Err(unavailable()),
    };
    if !status.is_success() {
        return Err((status, [(CONTENT_TYPE, "application/json")], bytes).into_response());
    }
    serde_json::from_slice(&bytes).map_err(|_| unavailable())
}

fn preparation_deadline(seconds: u32) -> Duration {
    Duration::from_secs(110 + u64::from(seconds)) + REQUEST_TIMEOUT
}

fn lifecycle_unavailable() -> Response {
    readiness_unavailable("verified lifecycle readiness unavailable")
}

fn environment_unavailable() -> Response {
    readiness_unavailable("environment readiness unavailable")
}

fn readiness_unavailable(message: &str) -> Response {
    (
        StatusCode::BAD_GATEWAY,
        axum::Json(serde_json::json!({"error":message})),
    )
        .into_response()
}

fn changed() -> Response {
    (
        StatusCode::CONFLICT,
        axum::Json(serde_json::json!({"error":"lifecycle selection changed; request a new readiness view"})),
    )
        .into_response()
}

#[cfg(test)]
mod tests;
