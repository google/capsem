//! KVM virtio-blk device metrics, labelled by `backend` (sync worker,
//! ioeventfd or io_uring).

use metrics::Unit;

use crate::MetricSpec;

pub const QUEUE_NOTIFICATIONS_TOTAL: &str = "virtio.blk.queue_notifications_total";
pub const QUEUE_DRAINS_TOTAL: &str = "virtio.blk.queue_drains_total";
pub const DESCRIPTORS_DRAINED_TOTAL: &str = "virtio.blk.descriptors_drained_total";
pub const USED_ENTRIES_TOTAL: &str = "virtio.blk.used_entries_total";
pub const INTERRUPTS_TOTAL: &str = "virtio.blk.interrupts_total";
pub const REQUESTS_TOTAL: &str = "virtio.blk.requests_total";
pub const REQUEST_BYTES_TOTAL: &str = "virtio.blk.request_bytes_total";
pub const REQUEST_DURATION_MS: &str = "virtio.blk.request_duration_ms";
pub const QUEUE_DRAIN_DURATION_MS: &str = "virtio.blk.queue_drain_duration_ms";
pub const QUIESCE_DRAIN_DURATION_MS: &str = "virtio.blk.quiesce_drain_duration_ms";
pub const ASYNC_SUBMISSIONS_TOTAL: &str = "virtio.blk.async_submissions_total";
pub const ASYNC_COMPLETIONS_TOTAL: &str = "virtio.blk.async_completions_total";
pub const ASYNC_FALLBACKS_TOTAL: &str = "virtio.blk.async_fallbacks_total";
pub const ASYNC_IN_FLIGHT: &str = "virtio.blk.async_in_flight";

pub const SPECS: &[MetricSpec] = &[
    MetricSpec::counter(
        QUEUE_NOTIFICATIONS_TOTAL,
        Unit::Count,
        "Virtio block queue notifications observed by backend.",
    ),
    MetricSpec::counter(
        QUEUE_DRAINS_TOTAL,
        Unit::Count,
        "Virtio block queue drain attempts by backend.",
    ),
    MetricSpec::counter(
        DESCRIPTORS_DRAINED_TOTAL,
        Unit::Count,
        "Virtio block descriptor chains drained by backend.",
    ),
    MetricSpec::counter(
        USED_ENTRIES_TOTAL,
        Unit::Count,
        "Virtio block used-ring entries published to the guest.",
    ),
    MetricSpec::counter(
        INTERRUPTS_TOTAL,
        Unit::Count,
        "Virtio block interrupt decisions, partitioned by raised|suppressed.",
    ),
    MetricSpec::counter(
        REQUESTS_TOTAL,
        Unit::Count,
        "Virtio block requests by operation and completion status.",
    ),
    MetricSpec::counter(
        REQUEST_BYTES_TOTAL,
        Unit::Bytes,
        "Virtio block request payload bytes by operation and completion status.",
    ),
    MetricSpec::histogram(
        REQUEST_DURATION_MS,
        Unit::Milliseconds,
        "Virtio block request processing wall time.",
    ),
    MetricSpec::histogram(
        QUEUE_DRAIN_DURATION_MS,
        Unit::Milliseconds,
        "Virtio block queue drain wall time per backend wake.",
    ),
    MetricSpec::histogram(
        QUIESCE_DRAIN_DURATION_MS,
        Unit::Milliseconds,
        "Virtio block quiesce drain wait time before checkpoint.",
    ),
    MetricSpec::counter(
        ASYNC_SUBMISSIONS_TOTAL,
        Unit::Count,
        "Virtio block io_uring submissions by operation.",
    ),
    MetricSpec::counter(
        ASYNC_COMPLETIONS_TOTAL,
        Unit::Count,
        "Virtio block io_uring completions by operation and completion status.",
    ),
    MetricSpec::counter(
        ASYNC_FALLBACKS_TOTAL,
        Unit::Count,
        "Virtio block requests handled by synchronous fallback from the async path.",
    ),
    MetricSpec::histogram(
        ASYNC_IN_FLIGHT,
        Unit::Count,
        "Virtio block io_uring in-flight request depth after submit/completion.",
    ),
];
