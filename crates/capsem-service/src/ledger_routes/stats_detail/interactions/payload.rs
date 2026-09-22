//! Decode bounded retained captures, never hot-path provider traffic.
use capsem_api::*;

fn raw_payload(raw: String, status: CaptureStatus, reason: RawContentReason) -> CapturedPayload {
    CapturedPayload {
        status,
        content: CapturedContent::Raw(RawContent {
            kind: RawContentKind::Raw,
            raw,
            reason,
        }),
    }
}

pub(super) fn json_payload(raw: String, status: CaptureStatus) -> CapturedPayload {
    if status == CaptureStatus::Truncated {
        return raw_payload(raw, status, RawContentReason::Truncated);
    }
    match serde_json::from_str(&raw) {
        Ok(value) => CapturedPayload {
            status,
            content: CapturedContent::Json(JsonContent {
                kind: JsonContentKind::Json,
                value,
            }),
        },
        Err(_) => raw_payload(raw, status, RawContentReason::InvalidJson),
    }
}

pub(super) fn text_payload(text: String) -> CapturedPayload {
    CapturedPayload {
        status: CaptureStatus::Unknown,
        content: CapturedContent::Text(TextContent {
            kind: TextContentKind::Text,
            text,
        }),
    }
}

pub(super) fn preview_payload(raw: String) -> CapturedPayload {
    // Previews can be plain text or arbitrary JSON. Never coerce a malformed
    // object/array into an apparently well-formed structured tool result.
    let payload = json_payload(raw.clone(), CaptureStatus::Unknown);
    if matches!(payload.content, CapturedContent::Json(_)) || raw.trim_start().starts_with(['{', '[']) {
        payload
    } else {
        text_payload(raw)
    }
}

#[cfg(test)]
mod tests;
