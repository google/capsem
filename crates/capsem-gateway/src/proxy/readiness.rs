//! Creation wait planning is a mutation-boundary read, never a polled fetch or
//! launch grant. The service still performs current admission on the mutation.
use super::*;

const METADATA_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_METADATA_BYTES: usize = 4096;

pub(super) async fn creation_deadline(
    state: &AppState,
    headers: &HeaderMap,
    bytes: &[u8],
    fallback: Duration,
) -> Result<Duration, Response> {
    let environment = {
        // Invalid payloads still reach the authoritative service validator.
        let Ok(request) = serde_json::from_slice::<capsem_api::ProvisionRequest>(bytes) else {
            return Ok(fallback);
        };
        if request.validate_environment().is_err() || request.attach {
            return Ok(fallback);
        }
        let Some(environment) = request.environment else {
            return Ok(fallback);
        };
        environment
    };
    let lookup = async {
        let uri = format!(
            "http://localhost/environments/{}/revisions/{}/readiness",
            environment.environment_id, environment.revision_id
        );
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
    let Ok(response) = serde_json::from_slice::<capsem_api::EnvironmentReadinessResponse>(&bytes) else {
        return Err(unavailable());
    };
    if response.environment != environment {
        return Err(unavailable());
    }
    Ok(preparation_deadline(response.startup_timeout_seconds))
}

fn preparation_deadline(seconds: u32) -> Duration {
    Duration::from_secs(110 + u64::from(seconds)) + REQUEST_TIMEOUT
}
fn unavailable() -> Response {
    (
        StatusCode::BAD_GATEWAY,
        axum::Json(serde_json::json!({"error":"environment readiness unavailable"})),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn approved_timeout_domain_is_preserved_without_overflow() {
        for (seconds, total) in [(0, 230_u64), (600, 830), (u32::MAX, 4_294_967_525)] {
            assert_eq!(preparation_deadline(seconds), Duration::from_secs(total));
        }
    }
}
