use std::collections::HashMap;

use crate::models::{HistoryLayerFilter, TimelineLayer};
use crate::Memory;

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
    pub vcpu: Option<u32>,
    pub memory: Option<Memory>,
    pub env: Option<HashMap<String, String>>,
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
