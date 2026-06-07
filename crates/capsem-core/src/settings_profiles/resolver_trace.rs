//! Resolver trace artifact: a deterministic, append-only log of
//! every operation that contributed to the materialized
//! `EffectiveVmSettings`. Persisted beside
//! `vm-effective-settings.toml` as `vm-effective-trace.json`,
//! so support bundles and debug reports can replay "why does
//! the final value at path P look like this?".

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use super::{Result, SettingsProfilesError};

pub const VM_EFFECTIVE_TRACE_FILENAME: &str = "vm-effective-trace.json";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ResolverTraceOperation {
    Set,
    Add,
    Remove,
    Replace,
    Lock,
    Forbid,
    Derive,
    Reject,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ResolverTraceSourceKind {
    Default,
    Profile,
    Corp,
    Derived,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResolverTraceEvent {
    pub step: u32,
    pub path: String,
    pub operation: ResolverTraceOperation,
    pub source_kind: ResolverTraceSourceKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_profile_id: Option<String>,
    pub source_label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<JsonValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<JsonValue>,
    #[serde(default)]
    pub locked: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResolverTrace {
    pub events: Vec<ResolverTraceEvent>,
}

impl ResolverTrace {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append `event` with `step` set to the trace's current
    /// length. Panicking on `u32` overflow is acceptable here
    /// because every plausible chain stays well under 2^32
    /// events.
    pub fn append(&mut self, mut event: ResolverTraceEvent) {
        event.step =
            u32::try_from(self.events.len()).expect("resolver trace event count fits in u32");
        self.events.push(event);
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Compact summary for status / debug surfaces. Records the
    /// total event count, the count of corp-attributed events
    /// (so callers can tell at a glance "did corp policy touch
    /// this VM?"), the last N events for human-readable
    /// inspection, and the list of paths that ended up locked
    /// or rejected.
    pub fn summary(&self, tail: usize) -> ResolverTraceSummary {
        let corp_event_count = self
            .events
            .iter()
            .filter(|event| event.source_kind == ResolverTraceSourceKind::Corp)
            .count();
        let locked_paths: Vec<String> = self
            .events
            .iter()
            .filter(|event| matches!(event.operation, ResolverTraceOperation::Lock) || event.locked)
            .map(|event| event.path.clone())
            .collect();
        let rejected_paths: Vec<String> = self
            .events
            .iter()
            .filter(|event| matches!(event.operation, ResolverTraceOperation::Reject))
            .map(|event| event.path.clone())
            .collect();
        let last_events: Vec<ResolverTraceEvent> = self
            .events
            .iter()
            .rev()
            .take(tail)
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        ResolverTraceSummary {
            event_count: self.events.len(),
            corp_event_count,
            locked_paths,
            rejected_paths,
            last_events,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResolverTraceSummary {
    pub event_count: usize,
    pub corp_event_count: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub locked_paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rejected_paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub last_events: Vec<ResolverTraceEvent>,
}

pub fn vm_effective_trace_path(session_dir: impl AsRef<Path>) -> PathBuf {
    session_dir.as_ref().join(VM_EFFECTIVE_TRACE_FILENAME)
}

pub fn load_vm_effective_trace(session_dir: impl AsRef<Path>) -> Result<ResolverTrace> {
    let path = vm_effective_trace_path(session_dir);
    let input = fs::read_to_string(&path).map_err(|source| SettingsProfilesError::ReadFile {
        path: path.clone(),
        details: source.to_string(),
    })?;
    serde_json::from_str::<ResolverTrace>(&input).map_err(|source| SettingsProfilesError::Parse {
        kind: "vm-effective trace",
        details: source.to_string(),
    })
}

pub fn write_vm_effective_trace(
    session_dir: impl AsRef<Path>,
    trace: &ResolverTrace,
) -> Result<PathBuf> {
    let path = vm_effective_trace_path(session_dir);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| SettingsProfilesError::WriteFile {
            path: parent.to_path_buf(),
            details: source.to_string(),
        })?;
    }
    let payload =
        serde_json::to_string_pretty(trace).map_err(|source| SettingsProfilesError::Serialize {
            kind: "vm-effective trace",
            details: source.to_string(),
        })?;
    fs::write(&path, payload).map_err(|source| SettingsProfilesError::WriteFile {
        path: path.clone(),
        details: source.to_string(),
    })?;
    Ok(path)
}

#[cfg(test)]
mod tests;
