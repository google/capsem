pub mod db;
pub mod events;
pub mod reader;
pub mod schema;
pub mod writer;

pub use db::SessionDb;
pub use events::{
    credential_reference, is_credential_reference, AuditEvent, Decision, DnsEvent, ExecEvent,
    ExecEventComplete, FileAction, FileEvent, McpCall, ModelCall, NetEvent, ProfileMutationEvent,
    ProfileMutationStatus, SecurityAskEvent, SecurityAskPending, SecurityAskStatus,
    SecurityDecision, SecurityDecisionEvent, SecurityDecisionStage, SecurityDetectionLevel,
    SecurityRuleAction, SecurityRuleEvent, SubstitutionEvent, ToolCallEntry, ToolResponseEntry,
    CREDENTIAL_REF_PREFIX,
};
pub use reader::{
    validate_select_only, DbReader, DomainCount, FileEventStats, HistoryCounts, HistoryEntry,
    McpCallStats, McpServerCallCount, McpToolUsage, NetEventCounts, ProcessEntry,
    ProviderTokenUsage, SecurityRuleActionCount, SecurityRuleDetectionLevelCount,
    SecurityRuleEventTypeCount, SecurityRuleStats, SecurityRuleStatsByRule, SessionStats,
    TimeBucket, ToolUsageCount, ToolUsageWithStats, TraceDetail, TraceModelCall, TraceSummary,
};
pub use writer::{DbWriter, WriteOp};
