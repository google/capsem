use std::time::Duration;

use http_body_util::{BodyExt, Full};
use hyper::body::{Body, Bytes};
use tracing::Instrument;

use super::body::ProxyBoxBody;

const BODY_LIMIT: usize = 16 * 1024 * 1024;
const DECOMPRESSED_BODY_LIMIT: usize = 32 * 1024 * 1024;
const BODY_DEADLINE: Duration = Duration::from_secs(60);

pub(super) fn retain_admission<B>(
    body: super::body::ChunkDispatchBody<B>,
    permit: Option<tokio::sync::OwnedSemaphorePermit>,
) -> super::body::ChunkDispatchBody<B> {
    if let Some(permit) = permit {
        body.seed(permit)
    } else {
        body
    }
}

/// Carry the actual pre-forwarding decision, never reconstruct a detector
/// result from a truncated or already-redacted telemetry preview.
pub(super) fn record_content_evidence(
    target: &mut Option<Box<crate::security_engine::SecurityEvent>>,
    event: &crate::security_engine::SecurityEvent,
) {
    let own = |id: &str| matches!(id, "pii_filter" | "secret_exfiltration" | "utf8_guard");
    let executions: Vec<_> = event
        .plugin_executions
        .iter()
        .filter(|e| own(&e.plugin_id))
        .cloned()
        .collect();
    if executions.is_empty() {
        return;
    }
    let target = target.get_or_insert_with(|| {
        Box::new(crate::security_engine::SecurityEvent::new(
            crate::security_engine::RuntimeSecurityEventType::HttpRequest,
        ))
    });
    for execution in executions {
        if let Some(previous) = target
            .plugin_executions
            .iter_mut()
            .find(|previous| previous.plugin_id == execution.plugin_id && previous.stage == execution.stage)
        {
            // One HTTP exchange can inspect both directions. Preserve a later
            // response match even when the request-side inspection was clean.
            previous.applied |= execution.applied;
            previous.duration_us = previous.duration_us.saturating_add(execution.duration_us);
        } else {
            target.plugin_executions.push(execution);
        }
    }
    target.detections.extend(
        event
            .detections
            .iter()
            .filter(|d| d.plugin_id.as_deref().is_some_and(own))
            .cloned(),
    );
    target.request_decision(event.decision.effective);
}

pub(super) fn has_content_match(event: Option<&crate::security_engine::SecurityEvent>) -> bool {
    event.is_some_and(|event| !event.detections.is_empty())
}

pub(super) fn inspection_enabled(snapshot: &crate::net::proxy_engine::ProxyPolicySnapshot, inbound: bool) -> bool {
    snapshot.plugins().iter().any(|(id, config)| {
        (id == "pii_filter" || (id == "secret_exfiltration" && !inbound))
            && config.mode != capsem_config::SecurityPluginMode::Disable
            && config.content.direction.includes(inbound)
    })
}

pub(super) fn utf8_enabled(
    snapshot: &crate::net::proxy_engine::ProxyPolicySnapshot,
    inbound: bool,
    headers: &http::HeaderMap,
) -> bool {
    let enabled = snapshot.plugins().get("utf8_guard").is_some_and(|config| {
        config.mode != capsem_config::SecurityPluginMode::Disable && config.content.direction.includes(inbound)
    });
    if !enabled {
        return false;
    }
    let Some(media_type) = headers
        .get(http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
    else {
        return true;
    };
    let media_type = media_type.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    media_type.starts_with("text/")
        || matches!(
            media_type.as_str(),
            "application/json" | "application/xml" | "application/javascript" | "application/x-www-form-urlencoded"
        )
        || media_type.ends_with("+json")
        || media_type.ends_with("+xml")
}

/// Body-inspection plugins are text-only. Unsupported encodings and invalid
/// UTF-8 fail closed, rather than scanning a lossy/truncated preview. Gzip is
/// decoded by the existing capped codec off the proxy's I/O thread.
pub(super) async fn inspection_body(
    body: Bytes,
    headers: &http::HeaderMap,
    require_utf8: bool,
) -> anyhow::Result<Bytes> {
    let encodings: Vec<_> = headers.get_all(http::header::CONTENT_ENCODING).iter().collect();
    let gzip = match encodings.as_slice() {
        [] => false,
        [value] if value.as_bytes().eq_ignore_ascii_case(b"identity") => false,
        [value] if value.as_bytes().eq_ignore_ascii_case(b"gzip") => true,
        _ => anyhow::bail!("content inspection refuses unsupported content encoding"),
    };
    let body = if gzip {
        tokio::task::spawn_blocking(move || decompress_gzip(body, true)).await??
    } else {
        body
    };
    if require_utf8 {
        std::str::from_utf8(&body).map_err(|_| anyhow::anyhow!("content inspection refuses non-UTF-8 body"))?;
    }
    Ok(body)
}

pub(super) fn replay_headers(headers: &mut http::HeaderMap, length: usize) {
    headers.remove(http::header::CONTENT_ENCODING);
    headers.remove(http::header::TRANSFER_ENCODING);
    headers.insert(
        http::header::CONTENT_LENGTH,
        http::HeaderValue::from_str(&length.to_string()).expect("bounded body length"),
    );
}

pub(super) enum UpstreamRequestBody {
    Replayable(Bytes),
    Streaming(ProxyBoxBody),
}

impl UpstreamRequestBody {
    pub(super) fn into_body(self) -> ProxyBoxBody {
        match self {
            Self::Replayable(body) => Full::new(body)
                .map_err(|never| -> anyhow::Error { match never {} })
                .boxed(),
            Self::Streaming(body) => body,
        }
    }
}

pub(super) async fn collect<B>(body: B, span: tracing::Span) -> Result<Bytes, String>
where
    B: Body<Data = Bytes> + Send + 'static,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    match tokio::time::timeout(
        BODY_DEADLINE,
        http_body_util::Limited::new(body, BODY_LIMIT)
            .collect()
            .instrument(span),
    )
    .await
    {
        Ok(Ok(collected)) => Ok(collected.to_bytes()),
        Ok(Err(error)) => Err(error.to_string()),
        Err(error) => Err(error.to_string()),
    }
}

pub(super) fn decompress_gzip(body: Bytes, is_gzip: bool) -> anyhow::Result<Bytes> {
    if !is_gzip {
        return Ok(body);
    }
    // A small compressed response can expand into a much larger policy event.
    let decompressed = crate::net::decompress::decompress_gzip_capped(&body, DECOMPRESSED_BODY_LIMIT)?;
    Ok(Bytes::from(decompressed))
}
