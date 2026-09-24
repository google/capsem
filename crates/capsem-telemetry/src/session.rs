//! Per-session totals, exported from each session's counter snapshot.
//!
//! Unlike the other domains these are not recorded through the facade: the
//! service observes them from the ledgers' counter snapshots on each export
//! (`capsem-service/src/telemetry_export.rs`). Every series carries exactly
//! `session.id`, `profile.id` and `persistent`; rule ids, domains, tools and
//! models stay in the API, where their cardinality is someone's choice.

use metrics::Unit;

use crate::MetricSpec;

pub const SESSION_REQUESTS_TOTAL: &str = "session.requests_total";
pub const SESSION_TOKENS_TOTAL: &str = "session.tokens_total";
pub const SESSION_COST_USD_TOTAL: &str = "session.cost_usd_total";
pub const SESSION_MODEL_CALLS_TOTAL: &str = "session.model_calls_total";
pub const SESSION_TOOL_CALLS_TOTAL: &str = "session.tool_calls_total";
pub const SESSION_FILE_EVENTS_TOTAL: &str = "session.file_events_total";
pub const SESSION_RULE_MATCHES_TOTAL: &str = "session.rule_matches_total";

pub const SPECS: &[MetricSpec] = &[
    MetricSpec::counter(
        SESSION_REQUESTS_TOTAL,
        Unit::Count,
        "Network requests the session made, by decision (allowed, denied, error).",
    ),
    MetricSpec::counter(
        SESSION_TOKENS_TOTAL,
        Unit::Count,
        "Model tokens the session used, by direction (input, output).",
    ),
    MetricSpec::new(
        SESSION_COST_USD_TOTAL,
        crate::MetricKind::Counter,
        None,
        "Estimated model cost of the session, in US dollars.",
    ),
    MetricSpec::counter(SESSION_MODEL_CALLS_TOTAL, Unit::Count, "Model calls the session made."),
    MetricSpec::counter(
        SESSION_TOOL_CALLS_TOTAL,
        Unit::Count,
        "Tool calls the session made, counting the same origins every API surface counts.",
    ),
    MetricSpec::counter(
        SESSION_FILE_EVENTS_TOTAL,
        Unit::Count,
        "File changes the session recorded, not counting watcher overflow markers.",
    ),
    MetricSpec::counter(
        SESSION_RULE_MATCHES_TOTAL,
        Unit::Count,
        "Security rule matches in the session.",
    ),
];
