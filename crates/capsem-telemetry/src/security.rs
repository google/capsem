//! Security engine metrics: audit-event admission and plugin execution.

use metrics::Unit;

use crate::MetricSpec;

pub const SECURITY_EVENT_EMIT_TOTAL: &str = "security_event.emit_total";
pub const SECURITY_EVENT_EMIT_DURATION_MS: &str = "security_event.emit_duration_ms";
pub const SECURITY_PLUGIN_EXECUTION_TOTAL: &str = "security_plugin.execution_total";
pub const SECURITY_PLUGIN_EXECUTION_DURATION_MS: &str = "security_plugin.execution_duration_ms";

pub const SPECS: &[MetricSpec] = &[
    MetricSpec::counter(
        SECURITY_EVENT_EMIT_TOTAL,
        Unit::Count,
        "Security audit events offered to the session ledger, by event type, family, status and queue result.",
    ),
    MetricSpec::histogram(
        SECURITY_EVENT_EMIT_DURATION_MS,
        Unit::Milliseconds,
        "Wall time to admit one security audit event to the ledger writer, by event type and family.",
    ),
    MetricSpec::counter(
        SECURITY_PLUGIN_EXECUTION_TOTAL,
        Unit::Count,
        "Security plugin executions, by plugin, stage, mode, whether it applied, and status.",
    ),
    MetricSpec::histogram(
        SECURITY_PLUGIN_EXECUTION_DURATION_MS,
        Unit::Milliseconds,
        "Wall time of one security plugin execution, by plugin, stage, mode, whether it applied, and status.",
    ),
];
