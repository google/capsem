//! Embedded Xpra clients authenticate their WebSocket independently of browser
//! third-party cookie policy. The single-use bootstrap is posted by a trusted
//! Capsem parent; the scoped credential is delivered only to pinned client code.
use super::{Head, Scope};

pub(super) const PATH: &str = "/_capsem/surface/index.html";

pub(super) fn trusted_parent(head: &Head) -> anyhow::Result<&str> {
    let origin = head
        .origin
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("embedded parent origin missing"))?;
    let header = http::HeaderValue::from_str(origin)?;
    anyhow::ensure!(
        crate::cors::is_allowed_origin(&header),
        "untrusted embedded parent origin"
    );
    Ok(origin)
}

pub(super) fn trusted_initiator(head: &Head, gateway_port: u16) -> anyhow::Result<&str> {
    let origin = head
        .origin
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("bootstrap initiator missing"))?;
    // Authority comes from our bound listener, never a caller-supplied Host
    // or arbitrary guest-published loopback port. The recipient remains a
    // separately validated Capsem parent, and receives only a scoped preview.
    anyhow::ensure!(
        origin == "tauri://localhost" || origin == format!("http://127.0.0.1:{gateway_port}"),
        "untrusted bootstrap initiator"
    );
    Ok(origin)
}

pub(super) fn parent_from_form(body: &str, head: &Head, gateway_port: u16) -> anyhow::Result<String> {
    trusted_initiator(head, gateway_port)?;
    let mut parents = url::form_urlencoded::parse(body.as_bytes())
        .filter_map(|(key, value)| (key == "parent_origin").then(|| value.into_owned()));
    let parent = match parents.next() {
        Some(parent) => parent,
        None => trusted_parent(head)?.to_owned(),
    };
    anyhow::ensure!(parents.next().is_none(), "duplicate embedded parent");
    anyhow::ensure!(
        crate::cors::is_allowed_origin(&http::HeaderValue::from_str(&parent)?),
        "untrusted embedding parent"
    );
    Ok(parent)
}

pub(super) fn parse_protocol_token(value: &str, token: &mut Option<String>) -> anyhow::Result<()> {
    for offered in value.split(',').map(str::trim) {
        if let Some(value) = offered.strip_prefix(capsem_proto::PREVIEW_WS_PROTOCOL_PREFIX) {
            anyhow::ensure!(token.is_none(), "duplicate embedded preview credential");
            anyhow::ensure!(
                !value.is_empty()
                    && value.len() <= 256
                    && value
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')),
                "invalid embedded preview credential"
            );
            *token = Some(value.to_owned());
        }
    }
    Ok(())
}

pub(super) fn session_token<'a>(head: &'a Head, scope: &Scope) -> anyhow::Result<Option<&'a str>> {
    let Some(token) = head.protocol_token.as_deref() else {
        return Ok(head.cookie.as_deref());
    };
    anyhow::ensure!(
        scope.surface && head.websocket && head.method == "GET",
        "embedded credential requires a surface WebSocket"
    );
    anyhow::ensure!(
        head.content_length == 0 && !head.transfer_encoded,
        "WebSocket requests carry no body"
    );
    let origin = format!("http://{}.localhost:{}", head.label, head.port);
    anyhow::ensure!(
        head.origin.as_deref() == Some(&origin),
        "surface WebSocket origin mismatch"
    );
    anyhow::ensure!(scope.issued(token), "unknown or expired embedded preview credential");
    // An old standalone cookie may coexist with a newly bootstrapped embedded
    // session. Explicit WebSocket credentials take precedence consistently.
    Ok(Some(token))
}

#[cfg(test)]
mod tests;
