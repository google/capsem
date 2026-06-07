//! First-byte protocol classification for the MITM listener.
//!
//! The vsock:5002 listener accepts whatever the guest's `net_proxy`
//! relays to it. Today that is TLS (port 443 redirect), plain HTTP/1.1
//! (port 80 + allowlist redirect, e.g. Ollama on 11434), and the T0
//! framed MCP wire-gate transport used to compare the future MITM MCP path.
//!
//! Distinguishing the two from the wire is a single-byte check
//! against the first payload byte the listener sees AFTER the optional
//! `\0CAPSEM_META:` process-name prefix is stripped:
//!
//! * `0x16` — TLS handshake record (ClientHello). All TLS sessions
//!   start with a Handshake record (RFC 8446 §5.1); the only valid
//!   first-byte from a client.
//! * Uppercase ASCII (`0x41..=0x5A`) — HTTP/1.1 request line. Every
//!   IETF-defined HTTP method starts with an uppercase letter
//!   (GET, POST, PUT, HEAD, DELETE, OPTIONS, PATCH, TRACE, CONNECT).
//!   Lowercase, digits, control characters, and high-bit bytes are not
//!   plausible HTTP method starts.
//! * Length-prefixed frame with `MC` magic — framed MCP JSON-RPC.
//!
//! The first-byte ranges do not overlap for TLS and HTTP; framed MCP
//! validates the four-byte length prefix plus the following two-byte magic.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Protocol {
    /// TLS (HTTPS): first byte is the TLS record type 0x16 (Handshake).
    Tls,
    /// Plain HTTP/1.1: first byte is an uppercase ASCII letter (HTTP
    /// method). T2.1 detects this; T2.2 wires the actual handler.
    Http,
    /// Framed MCP JSON-RPC over the MITM vsock port. This is the T0
    /// benchmark gate transport for the MCP unification sprint.
    McpFrame,
    /// Pre-classification default and unrecognized-byte fallback. The
    /// listener returns an error connection event when the wire payload
    /// matches neither TLS nor HTTP.
    #[default]
    Unknown,
}

impl Protocol {
    /// Stable label for `mitm.connections_total{protocol=…}` and
    /// `mitm.requests_total{protocol=…}`.
    pub fn label(self) -> &'static str {
        match self {
            Protocol::Tls => "tls",
            Protocol::Http => "http",
            Protocol::McpFrame => "mcp-frame",
            Protocol::Unknown => "unknown",
        }
    }
}

/// Classify the protocol from the post-meta payload.
/// `None` for empty input or an unrecognized first byte.
pub fn detect(buf: &[u8]) -> Option<Protocol> {
    let first = *buf.first()?;
    if first == 0x16 {
        return Some(Protocol::Tls);
    }
    if first.is_ascii_uppercase() {
        return Some(Protocol::Http);
    }
    if capsem_proto::looks_like_mcp_frame_prefix(buf) {
        return Some(Protocol::McpFrame);
    }
    None
}

#[cfg(test)]
mod tests;
