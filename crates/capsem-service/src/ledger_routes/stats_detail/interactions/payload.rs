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

pub(super) fn body_payload(body: &EventBody) -> CapturedPayload {
    if body.truncated {
        return raw_payload(body.body.clone(), CaptureStatus::Truncated, RawContentReason::Truncated);
    }
    let content_type = body
        .content_type
        .as_deref()
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if content_type == "application/json" || content_type.ends_with("+json") {
        json_payload(body.body.clone(), CaptureStatus::Complete)
    } else if content_type.starts_with("text/") && content_type != "text/event-stream" {
        let mut payload = text_payload(body.body.clone());
        payload.status = CaptureStatus::Complete;
        payload
    } else {
        raw_payload(body.body.clone(), CaptureStatus::Complete, RawContentReason::Unparsed)
    }
}

#[cfg(test)]
mod tests;
