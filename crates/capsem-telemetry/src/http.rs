//! Normalized server-side HTTP lifecycle metrics; never client wire or CPU clocks.

pub mod operations;

use crate::MetricSpec;
use metrics::Unit;

pub const REQUESTS_TOTAL: &str = "http.server.requests_total";
pub const RESPONSE_READY_MS: &str = "http.server.response_ready_ms";
pub const BODY_LIFETIME_MS: &str = "http.server.body_lifetime_ms";
pub const RESPONSES_TOTAL: &str = "http.server.responses_total";

pub const SPECS: &[MetricSpec] = &[
    MetricSpec::counter(
        REQUESTS_TOTAL,
        Unit::Count,
        "HTTP requests received by normalized component, method and route.",
    ),
    MetricSpec::histogram(
        RESPONSE_READY_MS,
        Unit::Milliseconds,
        "Server request start to response readiness, not client first-byte latency.",
    ),
    MetricSpec::histogram(
        BODY_LIFETIME_MS,
        Unit::Milliseconds,
        "Server response readiness to body termination, not client completion or CPU time.",
    ),
    MetricSpec::counter(
        RESPONSES_TOTAL,
        Unit::Count,
        "HTTP terminations by component, normalized operation, status and exact completion kind.",
    ),
];

#[cfg(feature = "http-observation")]
mod observation;
#[cfg(feature = "http-observation")]
pub use observation::{observe, Component};
