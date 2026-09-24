//! What the builtin MCP server hands capsem-process to record.
//!
//! One process writes a session's ledger, and it is capsem-process. The
//! builtin server runs tools whose effects only it can see -- the HTTP
//! requests it makes from the host -- so it describes each one as a record under a reserved `_meta` key of its tool
//! result. The aggregator passes a tool result through whole, and
//! capsem-process writes the records it finds there.
//!
//! The builtin used to open its own writer on the session ledger instead.
//! SQLite serialised the two writers, but the body archive beside it cannot:
//! each writer appends at the file's real end while numbering offsets from a
//! private counter, so the first block the second one sealed moved every
//! later offset the first one handed out.
//!
//! The key is honoured only on results from the server whose definition says
//! `source == "builtin"`, and stripped from every result whatever its source,
//! so no other server can write the ledger through it and the guest never
//! sees it.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The `_meta` key a builtin tool result carries its ledger records under.
pub const BUILTIN_LEDGER_META_KEY: &str = "dev.capsem/ledger";

/// One thing the builtin server did that the session ledger must show.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BuiltinLedgerRecord {
    /// An HTTP request a builtin tool made from the host, or refused to.
    HttpRequest(HttpRequestRecord),
}

/// Whether the security engine let a builtin HTTP request through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HttpDecision {
    Allowed,
    Denied,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpRequestRecord {
    /// When the builtin server finished with the request, not when
    /// capsem-process got around to recording it.
    pub timestamp_unix_ms: u64,
    pub domain: String,
    pub method: String,
    pub path: String,
    pub decision: HttpDecision,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_code: Option<u16>,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub duration_ms: u64,
    /// The enforcement action the security engine chose: `allow`, `ask` or
    /// `block`.
    pub policy_action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_rule: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_reason: Option<String>,
}

/// The value to store under [`BUILTIN_LEDGER_META_KEY`].
pub fn encode(records: &[BuiltinLedgerRecord]) -> Value {
    serde_json::to_value(records).expect("builtin ledger records are plain data and always serialize")
}

/// The records a value stored under [`BUILTIN_LEDGER_META_KEY`] holds.
pub fn decode(value: Value) -> Result<Vec<BuiltinLedgerRecord>, serde_json::Error> {
    serde_json::from_value(value)
}

/// Remove [`BUILTIN_LEDGER_META_KEY`] from a tool result and return what it
/// held.
///
/// A `_meta` left empty is removed too, so a stripped result is exactly the
/// result the server would have sent without the key.
pub fn take(result: &mut Value) -> Option<Value> {
    let result = result.as_object_mut()?;
    let meta = result.get_mut("_meta")?.as_object_mut()?;
    let taken = meta.remove(BUILTIN_LEDGER_META_KEY)?;
    if meta.is_empty() {
        result.remove("_meta");
    }
    Some(taken)
}

#[cfg(test)]
mod tests;
