use std::collections::HashMap;

use crate::models::{HistoryLayerFilter, NetworkInfo, RegistryAccess, TimelineLayer};

/// Exactly one way to select a VM; names resolve once through the gateway.
#[derive(Debug, Clone)]
pub enum VmSelector {
    Id(String),
    Name(String),
}

/// Omitted CPU and memory values retain the selected profile's defaults.
#[derive(Debug, Clone, Default)]
pub struct CreateOptions {
    pub name: Option<String>,
    pub cpus: Option<u32>,
    /// Guest memory in GiB.
    pub memory: Option<u64>,
    pub env: Option<HashMap<String, String>>,
    pub networks: Vec<NetworkInfo>,
    pub image: Option<String>,
    pub command: Vec<String>,
    pub registry: Option<RegistryAccess>,
    pub attach: bool,
}

#[derive(Debug, Clone, Default)]
pub struct RunOptions {
    pub profile: Option<String>,
    pub timeout_secs: Option<u64>,
    pub cpus: Option<u32>,
    /// Guest memory in GiB.
    pub memory: Option<u64>,
    pub env: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Default)]
pub struct DiagnosticOptions {
    pub since: Option<String>,
    pub limit: Option<u64>,
}

#[derive(Debug, Clone, Default)]
pub struct TriageOptions {
    pub since: Option<String>,
    pub limit: Option<u64>,
    pub vm_id: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct LogOptions {
    pub grep: Option<String>,
    pub tail: Option<u64>,
    pub max_bytes: Option<u64>,
}

#[derive(Debug, Clone, Default)]
pub struct HistoryOptions {
    pub limit: Option<u64>,
    pub offset: Option<u64>,
    pub search: Option<String>,
    pub layer: Option<HistoryLayerFilter>,
}

#[derive(Debug, Clone, Default)]
pub struct TimelineOptions {
    pub trace_id: Option<String>,
    pub since: Option<String>,
    pub limit: Option<u64>,
    pub layers: Option<Vec<TimelineLayer>>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct PageOptions {
    pub limit: Option<u64>,
    pub offset: Option<u64>,
}

#[derive(Debug, Clone, Default)]
pub struct NetworkLogOptions {
    pub cursor: Option<String>,
    pub limit: Option<u64>,
    pub vm: Option<String>,
    pub connection: Option<String>,
    pub event_type: Option<String>,
    pub decision: Option<String>,
    pub since: Option<i64>,
    pub until: Option<i64>,
}
