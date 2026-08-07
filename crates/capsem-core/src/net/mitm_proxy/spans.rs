//! Stable debug span names for the MITM/network lab.
//!
//! Span fields must stay low-cardinality. Do not add raw hostnames, paths,
//! URLs, headers, bodies, cookies, API keys, OAuth tokens, or credentials.

pub const MITM_CONNECTION: &str = "capsem.mitm.connection";
pub const MITM_REQUEST: &str = "capsem.mitm.request";
pub const MITM_VSOCK_CLASSIFY: &str = "capsem.mitm.vsock_classify";
pub const MITM_TLS_GUEST_HANDSHAKE: &str = "capsem.mitm.tls_guest_handshake";
pub const MITM_POLICY_REQUEST: &str = "capsem.mitm.policy.request";
pub const MITM_SECURITY_ACTIONS: &str = "capsem.mitm.security_actions";
pub const MITM_MODEL_REQUEST_POLICY: &str = "capsem.mitm.model.request_policy";
pub const MITM_UPSTREAM_PREPARE: &str = "capsem.mitm.upstream.prepare";
pub const MITM_UPSTREAM_SEND: &str = "capsem.mitm.upstream.send";
pub const MITM_POLICY_RESPONSE: &str = "capsem.mitm.policy.response";
pub const MITM_MODEL_RESPONSE_POLICY: &str = "capsem.mitm.model.response_policy";
pub const MITM_BODY_CHUNK_HOOKS: &str = "capsem.mitm.body.chunk_hooks";
pub const MITM_WEBSOCKET: &str = "capsem.mitm.websocket";
pub const MITM_TELEMETRY_EMIT: &str = "capsem.mitm.telemetry.emit";

#[cfg(test)]
mod tests;
