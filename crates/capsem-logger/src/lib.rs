pub mod counters;
pub mod db;
pub mod events;
pub mod host_admission;
pub mod ledger_client;
pub mod ledger_protocol;
pub mod ledger_server;
pub mod network_db;
pub mod reader;
pub mod schema;
pub mod session_types;
mod wire_bytes;
pub mod writer;

pub use db::{
    snapshot_session_ledger, ArchivedBodies, BodyDirection, DbHandle, ExportSummary, ReadCacheDomain, SessionDb,
    SkipReason, SkippedBody, StoredBody,
};
pub use events::{
    credential_reference, is_credential_reference, AuditEvent, Decision, DnsEvent, ExecEvent, ExecEventComplete,
    FileAction, FileEvent, FileKind, HostEvent, HostEventKind, McpCall, MembershipState, ModelCall, NetEvent,
    NetworkDeparture, NetworkMembership, NetworkRecord, NetworkState, PolicyMutationEvent, PolicyMutationStatus,
    SecurityAskEvent, SecurityAskPending, SecurityAskRecord, SecurityAskStatus, SecurityAskView, SecurityDecision,
    SecurityDecisionEvent, SecurityDecisionStage, SecurityDetectionLevel, SecurityRuleAction, SecurityRuleEvent,
    SecurityRuleMatch, SubstitutionEvent, ToolCallEntry, ToolResponseEntry, TransportEvent, TransportEventKind,
    CREDENTIAL_REF_PREFIX,
};
pub use reader::{
    validate_select_only, DbReader, HistoryEntry, ProcessEntry, SecurityRuleActionCount,
    SecurityRuleDetectionLevelCount, SecurityRuleEventTypeCount, SecurityRuleStats, SecurityRuleStatsByRule,
};
pub use session_types::{epoch_to_iso, generate_session_id, is_valid_session_id, now_iso};
pub use writer::{format_ledger_timestamp, output_preview, DbWriter, RetainOutcome, WriteOp, MAX_BODY_BLOB_BYTES};

/// Non-security stored-content fingerprint. The current schema accepts only
/// this explicit algorithm identity; producers never compute admission hashes.
pub(crate) fn content_digest(bytes: &[u8]) -> String {
    format!("xxh3-128:{:032x}", capsem_crypto::xxh3::hash(bytes))
}
