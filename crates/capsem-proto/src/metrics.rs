use serde::{Deserialize, Serialize};

pub const METRICS_SCHEMA_VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct VmMetricsSnapshot {
    pub schema_version: u32,
    pub vm_id: String,
    pub persistent: bool,
    pub lifecycle: VmLifecycleMetrics,
    pub resources: VmResourceMetrics,
    pub ask: VmAskMetrics,
    pub http: VmHttpMetrics,
    pub dns: VmDnsMetrics,
    pub model: VmModelMetrics,
    pub mcp: VmMcpMetrics,
    pub filesystem: VmFilesystemMetrics,
    pub process: VmProcessMetrics,
    pub hypervisor: VmHypervisorMetrics,
    pub security: VmSecurityMetrics,
    pub captured_at_unix_ms: u64,
}

impl VmMetricsSnapshot {
    pub fn empty(vm_id: impl Into<String>, persistent: bool, captured_at_unix_ms: u64) -> Self {
        Self {
            schema_version: METRICS_SCHEMA_VERSION,
            vm_id: vm_id.into(),
            persistent,
            lifecycle: VmLifecycleMetrics::default(),
            resources: VmResourceMetrics::default(),
            ask: VmAskMetrics::default(),
            http: VmHttpMetrics::default(),
            dns: VmDnsMetrics::default(),
            model: VmModelMetrics::default(),
            mcp: VmMcpMetrics::default(),
            filesystem: VmFilesystemMetrics::default(),
            process: VmProcessMetrics::default(),
            hypervisor: VmHypervisorMetrics::default(),
            security: VmSecurityMetrics::default(),
            captured_at_unix_ms,
        }
    }

    pub fn otel_metric_points(&self) -> Vec<OtelMetricPoint> {
        let mut points = Vec::new();
        let source = self.otel_point_source();
        self.push_resource_otel_points(&mut points, source);
        self.push_hypervisor_otel_points(&mut points, source);
        points
    }

    fn otel_point_source(&self) -> OtelPointSource<'_> {
        OtelPointSource {
            source_vm_id: &self.vm_id,
            persistent: self.persistent,
            captured_at_unix_ms: self.captured_at_unix_ms,
        }
    }

    fn push_resource_otel_points(
        &self,
        points: &mut Vec<OtelMetricPoint>,
        source: OtelPointSource<'_>,
    ) {
        let resource_attrs = vec![OtelMetricAttribute::new("component", "resource")];
        push_gauge(
            points,
            "capsem.vm.resource.configured_ram",
            "MiBy",
            self.resources.configured_ram_mb as f64,
            resource_attrs.clone(),
            source,
        );
        push_gauge(
            points,
            "capsem.vm.resource.configured_vcpus",
            "1",
            self.resources.configured_vcpus as f64,
            resource_attrs.clone(),
            source,
        );
        if let Some(bytes) = self.resources.host_process_rss_bytes {
            push_gauge(
                points,
                "capsem.vm.resource.host_process_rss",
                "By",
                bytes as f64,
                resource_attrs.clone(),
                source,
            );
        }
        if let Some(micros) = self.resources.host_cpu_time_micros {
            push_counter(
                points,
                "capsem.vm.resource.host_cpu_time",
                "us",
                micros as f64,
                resource_attrs.clone(),
                source,
            );
        }
    }

    fn push_hypervisor_otel_points(
        &self,
        points: &mut Vec<OtelMetricPoint>,
        source: OtelPointSource<'_>,
    ) {
        let block = &self.hypervisor.block;
        let attrs = vec![
            OtelMetricAttribute::new("component", "kvm_virtio_blk"),
            OtelMetricAttribute::new("backend", "aggregate"),
        ];
        push_counter(
            points,
            "capsem.vm.block.queue_notifications",
            "1",
            block.queue_notifications_total as f64,
            attrs.clone(),
            source,
        );
        push_counter(
            points,
            "capsem.vm.block.queue_drains",
            "1",
            block.queue_drains_total as f64,
            attrs.clone(),
            source,
        );
        push_counter(
            points,
            "capsem.vm.block.descriptors_drained",
            "1",
            block.descriptors_drained_total as f64,
            attrs.clone(),
            source,
        );
        push_counter(
            points,
            "capsem.vm.block.used_entries",
            "1",
            block.used_entries_total as f64,
            attrs.clone(),
            source,
        );
        push_counter(
            points,
            "capsem.vm.block.interrupts_raised",
            "1",
            block.interrupts_raised_total as f64,
            attrs.clone(),
            source,
        );
        push_counter(
            points,
            "capsem.vm.block.interrupts_suppressed",
            "1",
            block.interrupts_suppressed_total as f64,
            attrs.clone(),
            source,
        );
        push_counter(
            points,
            "capsem.vm.block.read_ops",
            "1",
            block.read_ops_total as f64,
            attrs.clone(),
            source,
        );
        push_counter(
            points,
            "capsem.vm.block.write_ops",
            "1",
            block.write_ops_total as f64,
            attrs.clone(),
            source,
        );
        push_counter(
            points,
            "capsem.vm.block.bytes_read",
            "By",
            block.bytes_read_total as f64,
            attrs.clone(),
            source,
        );
        push_counter(
            points,
            "capsem.vm.block.bytes_written",
            "By",
            block.bytes_written_total as f64,
            attrs.clone(),
            source,
        );
        push_counter(
            points,
            "capsem.vm.block.requests",
            "1",
            block.requests_total as f64,
            attrs.clone(),
            source,
        );
        push_counter(
            points,
            "capsem.vm.block.request_bytes",
            "By",
            block.request_bytes_total as f64,
            attrs.clone(),
            source,
        );
        push_counter(
            points,
            "capsem.vm.block.request_duration",
            "us",
            block.request_duration_micros_total as f64,
            attrs.clone(),
            source,
        );
        push_counter(
            points,
            "capsem.vm.block.queue_drain_duration",
            "us",
            block.queue_drain_duration_micros_total as f64,
            attrs.clone(),
            source,
        );
        push_gauge(
            points,
            "capsem.vm.block.max_request_bytes",
            "By",
            block.max_request_bytes as f64,
            attrs.clone(),
            source,
        );
        push_gauge(
            points,
            "capsem.vm.block.max_data_descriptors_per_request",
            "1",
            block.max_data_descriptors_per_request as f64,
            attrs.clone(),
            source,
        );
        push_gauge(
            points,
            "capsem.vm.block.max_requests_per_drain",
            "1",
            block.max_requests_per_drain as f64,
            attrs.clone(),
            source,
        );
        push_counter(
            points,
            "capsem.vm.block.async_submissions",
            "1",
            block.async_submissions_total as f64,
            attrs.clone(),
            source,
        );
        push_counter(
            points,
            "capsem.vm.block.async_completions",
            "1",
            block.async_completions_total as f64,
            attrs.clone(),
            source,
        );
        push_counter(
            points,
            "capsem.vm.block.async_fallbacks",
            "1",
            block.async_fallbacks_total as f64,
            attrs.clone(),
            source,
        );
        push_counter(
            points,
            "capsem.vm.block.async_queue_full",
            "1",
            block.async_queue_full_total as f64,
            attrs.clone(),
            source,
        );
        push_gauge(
            points,
            "capsem.vm.block.async_in_flight",
            "1",
            block.async_in_flight as f64,
            attrs,
            source,
        );
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OtelMetricKind {
    Counter,
    Gauge,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct OtelMetricPoint {
    pub name: String,
    pub unit: String,
    pub kind: OtelMetricKind,
    pub value: f64,
    pub attributes: Vec<OtelMetricAttribute>,
    pub source_vm_id: String,
    pub persistent: bool,
    pub captured_at_unix_ms: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct OtelMetricAttribute {
    pub key: String,
    pub value: String,
}

impl OtelMetricAttribute {
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
        }
    }
}

#[derive(Clone, Copy)]
struct OtelPointSource<'a> {
    source_vm_id: &'a str,
    persistent: bool,
    captured_at_unix_ms: u64,
}

fn push_counter(
    points: &mut Vec<OtelMetricPoint>,
    name: &str,
    unit: &str,
    value: f64,
    attributes: Vec<OtelMetricAttribute>,
    source: OtelPointSource<'_>,
) {
    push_point(
        points,
        name,
        unit,
        OtelMetricKind::Counter,
        value,
        attributes,
        source,
    );
}

fn push_gauge(
    points: &mut Vec<OtelMetricPoint>,
    name: &str,
    unit: &str,
    value: f64,
    attributes: Vec<OtelMetricAttribute>,
    source: OtelPointSource<'_>,
) {
    push_point(
        points,
        name,
        unit,
        OtelMetricKind::Gauge,
        value,
        attributes,
        source,
    );
}

fn push_point(
    points: &mut Vec<OtelMetricPoint>,
    name: &str,
    unit: &str,
    kind: OtelMetricKind,
    value: f64,
    attributes: Vec<OtelMetricAttribute>,
    source: OtelPointSource<'_>,
) {
    points.push(OtelMetricPoint {
        name: name.to_string(),
        unit: unit.to_string(),
        kind,
        value,
        attributes,
        source_vm_id: source.source_vm_id.to_string(),
        persistent: source.persistent,
        captured_at_unix_ms: source.captured_at_unix_ms,
    });
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct VmLifecycleMetrics {
    pub state: String,
    pub uptime_secs: u64,
    pub boot_count: u64,
    pub restart_count: u64,
    pub suspend_count: u64,
    pub resume_count: u64,
    pub shutdown_count: u64,
    pub unexpected_exit_count: u64,
    pub last_transition_unix_ms: Option<u64>,
    pub last_error: Option<String>,
}

impl Default for VmLifecycleMetrics {
    fn default() -> Self {
        Self {
            state: "unknown".to_string(),
            uptime_secs: 0,
            boot_count: 0,
            restart_count: 0,
            suspend_count: 0,
            resume_count: 0,
            shutdown_count: 0,
            unexpected_exit_count: 0,
            last_transition_unix_ms: None,
            last_error: None,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
pub struct VmResourceMetrics {
    pub configured_ram_mb: u64,
    pub configured_vcpus: u32,
    pub host_pid: Option<u32>,
    pub host_process_rss_bytes: Option<u64>,
    pub host_cpu_time_micros: Option<u64>,
    pub host_cpu_percent: Option<f64>,
    pub session_disk_bytes: Option<u64>,
    pub workspace_disk_bytes: Option<u64>,
    pub rootfs_overlay_bytes: Option<u64>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct VmAskMetrics {
    pub total_asks: u64,
    pub asks_allowed: u64,
    pub asks_denied: u64,
    pub asks_errored: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct VmHttpMetrics {
    pub http_requests_total: u64,
    pub http_requests_allowed_total: u64,
    pub http_requests_warned_total: u64,
    pub http_requests_denied_total: u64,
    pub http_requests_errored_total: u64,
    pub http_bytes_sent_total: u64,
    pub http_bytes_received_total: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct VmDnsMetrics {
    pub dns_queries_total: u64,
    pub dns_queries_allowed_total: u64,
    pub dns_queries_warned_total: u64,
    pub dns_queries_denied_total: u64,
    pub dns_queries_rewritten_total: u64,
    pub dns_queries_errored_total: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct VmModelMetrics {
    pub model_requests_total: u64,
    pub model_requests_allowed_total: u64,
    pub model_requests_warned_total: u64,
    pub model_requests_denied_total: u64,
    pub model_requests_errored_total: u64,
    pub model_input_tokens_total: u64,
    pub model_output_tokens_total: u64,
    pub model_estimated_cost_micros_total: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct VmMcpMetrics {
    pub mcp_tool_invocations_total: u64,
    pub mcp_tool_invocations_allowed_total: u64,
    pub mcp_tool_invocations_warned_total: u64,
    pub mcp_tool_invocations_denied_total: u64,
    pub mcp_tool_invocations_errored_total: u64,
    pub mcp_servers_connected_total: u64,
    pub mcp_servers_disconnected_total: u64,
    pub mcp_server_errors_total: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct VmFilesystemMetrics {
    pub fs_reads_total: u64,
    pub fs_writes_total: u64,
    pub fs_creates_total: u64,
    pub fs_deletes_total: u64,
    pub fs_restores_total: u64,
    pub fs_errors_total: u64,
    pub fs_bytes_read_total: u64,
    pub fs_bytes_written_total: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct VmProcessMetrics {
    pub process_events_total: u64,
    pub process_exec_total: u64,
    pub process_audit_total: u64,
    pub process_errors_total: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct VmHypervisorMetrics {
    pub block: VmBlockMetrics,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct VmBlockMetrics {
    pub queue_notifications_total: u64,
    pub queue_drains_total: u64,
    pub descriptors_drained_total: u64,
    pub used_entries_total: u64,
    pub interrupts_raised_total: u64,
    pub interrupts_suppressed_total: u64,
    pub read_ops_total: u64,
    pub write_ops_total: u64,
    pub bytes_read_total: u64,
    pub bytes_written_total: u64,
    pub requests_total: u64,
    pub request_bytes_total: u64,
    pub request_duration_micros_total: u64,
    pub queue_drain_duration_micros_total: u64,
    pub max_request_bytes: u64,
    pub max_data_descriptors_per_request: u64,
    pub max_requests_per_drain: u64,
    pub async_submissions_total: u64,
    pub async_completions_total: u64,
    pub async_fallbacks_total: u64,
    pub async_queue_full_total: u64,
    pub async_in_flight: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct VmSecurityMetrics {
    pub security_events_total: u64,
    pub enforcement_decisions_total: u64,
    pub detection_findings_total: u64,
    pub blocks_total: u64,
    pub asks_total: u64,
    pub rewrites_total: u64,
    pub throttles_total: u64,
    pub errors_total: u64,
    pub latest_block_event_id: Option<String>,
    pub latest_block_rule_id: Option<String>,
    pub latest_block_reason: Option<String>,
    pub latest_block_unix_ms: Option<u64>,
    pub latest_detection_event_id: Option<String>,
    pub latest_detection_rule_id: Option<String>,
    pub latest_detection_title: Option<String>,
    pub latest_detection_severity: Option<String>,
    pub latest_detection_unix_ms: Option<u64>,
}

#[cfg(test)]
mod tests;
