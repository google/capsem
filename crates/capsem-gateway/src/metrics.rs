//! OTel-ready gateway control-plane metric names.
//!
//! These counters explain `/status` polling and service fan-out without
//! requiring an exporter in this slice. The metrics facade is a no-op unless a
//! recorder is installed.

use std::sync::Once;

use metrics::{describe_counter, describe_histogram, Unit};

pub const STATUS_CACHE_TOTAL: &str = "gateway.status.cache_total";
pub const STATUS_REFRESH_TOTAL: &str = "gateway.status.refresh_total";
pub const STATUS_REFRESH_DURATION_MS: &str = "gateway.status.refresh_duration_ms";
pub const STATUS_SERVICE_REQUESTS_TOTAL: &str = "gateway.status.service_requests_total";

static DESCRIBE: Once = Once::new();

pub fn describe_all() {
    DESCRIBE.call_once(|| {
        describe_counter!(
            STATUS_CACHE_TOTAL,
            Unit::Count,
            "Gateway /status cache decisions, partitioned by state hit|miss|stale|refreshed_by_peer."
        );
        describe_counter!(
            STATUS_REFRESH_TOTAL,
            Unit::Count,
            "Gateway /status refreshes, partitioned by result running|unavailable."
        );
        describe_histogram!(
            STATUS_REFRESH_DURATION_MS,
            Unit::Milliseconds,
            "Wall time spent refreshing gateway /status from capsem-service."
        );
        describe_counter!(
            STATUS_SERVICE_REQUESTS_TOTAL,
            Unit::Count,
            "Gateway /status service fan-out requests, partitioned by endpoint list|info and result ok|error."
        );
    });
}
