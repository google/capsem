//! The wire shape of an archived request, response or payload body.
//!
//! One shape for every caller. The stats view wants a preview to render and a
//! forensic client wants the bytes; both get the same object, because a body
//! that is described differently depending on who asked is a body no one can
//! compare across two tools.
//!
//! Three numbers, and they mean three different things:
//!
//! - `original_bytes` is what the upstream sent. Nothing Capsem did reduces
//!   it, so a reader can always say how much of the real body they have.
//! - `stored_bytes` is what the archive kept. `truncated` says the capture was
//!   cut -- upstream sent more than the capture cap.
//! - `content` is what this response carries. `truncated_for_transport` says
//!   *the route* cut it to the caller's byte budget, which is a different
//!   statement from `truncated` and has to stay one: a UI that conflated them
//!   would tell a reviewer the evidence is incomplete when the whole body is
//!   sitting in the archive one larger request away.
//!
//! When #199 lands these types move to `capsem-api` and are generated into the
//! TypeScript SDK; the frontend mirrors them by hand until then.

use serde::{Deserialize, Serialize};

/// How [`EventBody::content`] is encoded.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BodyEncoding {
    /// The bytes were valid UTF-8 and `content` is the text itself.
    Utf8,
    /// The bytes were not valid UTF-8 and `content` is standard base64.
    Base64,
}

/// Every archived body of one event.
///
/// An event with no archived body is an empty list, not a 404: the event may
/// exist and simply have had no body, and that is an answer rather than an
/// error.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct EventBodiesResponse {
    pub event_id: String,
    pub bodies: Vec<EventBody>,
}

/// One archived body, bounded to what the route agreed to send.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct EventBody {
    pub event_id: String,
    /// The ledger table the event lives in: `net_events`, `model_calls`,
    /// `tool_calls`, `security_rule_events`.
    pub source_table: String,
    /// `request`, `response`, `payload`, `stdout` or `stderr`.
    pub direction: String,
    pub content_type: Option<String>,
    /// What the upstream sent, whatever anything downstream kept.
    pub original_bytes: u64,
    /// What the archive holds.
    pub stored_bytes: u64,
    /// The capture was cut: upstream sent more than the capture cap.
    pub truncated: bool,
    /// This response was cut to the caller's byte budget. Independent of
    /// `truncated`; either, neither or both may be true.
    pub truncated_for_transport: bool,
    pub body_hash: String,
    pub encoding: BodyEncoding,
    pub content: String,
}
