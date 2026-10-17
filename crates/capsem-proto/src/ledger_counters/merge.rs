//! Folding counter snapshots: the host's totals are the sum of its sessions'.
//!
//! Counts add, maxima take the greater, ranges widen, and "latest" values
//! follow their timestamp, so a merged snapshot means what a single ledger
//! would have recorded had every row landed in it. Every map key goes through
//! `bounded_entry`, so the per-map bound and its overflow folding still hold.

use super::*;

trait Absorb {
    fn absorb(&mut self, other: &Self);
}

impl Absorb for u64 {
    fn absorb(&mut self, other: &Self) {
        *self = self.saturating_add(*other);
    }
}

impl<V: Absorb + Default> Absorb for BTreeMap<String, V> {
    fn absorb(&mut self, other: &Self) {
        for (key, value) in other {
            bounded_entry(self, key).absorb(value);
        }
    }
}

impl LedgerCounters {
    /// Fold `other` into this snapshot.
    pub fn absorb(&mut self, other: &Self) {
        Absorb::absorb(self, other);
    }
}

impl Absorb for LedgerCounters {
    fn absorb(&mut self, other: &Self) {
        self.net.absorb(&other.net);
        self.model.absorb(&other.model);
        self.tools.absorb(&other.tools);
        self.files.absorb(&other.files);
        self.exec.absorb(&other.exec);
        self.audit.absorb(&other.audit);
        self.security.absorb(&other.security);
        self.plugins.absorb(&other.plugins);
        self.credentials.absorb(&other.credentials);
    }
}

impl Absorb for NetCounters {
    fn absorb(&mut self, other: &Self) {
        self.total.absorb(&other.total);
        self.allowed.absorb(&other.allowed);
        self.denied.absorb(&other.denied);
        self.error.absorb(&other.error);
        self.bytes_sent.absorb(&other.bytes_sent);
        self.bytes_received.absorb(&other.bytes_received);
    }
}

impl Absorb for ModelUsage {
    fn absorb(&mut self, other: &Self) {
        self.calls.absorb(&other.calls);
        self.input_tokens.absorb(&other.input_tokens);
        self.output_tokens.absorb(&other.output_tokens);
        self.duration_ms.absorb(&other.duration_ms);
        self.cost_micro_usd.absorb(&other.cost_micro_usd);
    }
}

impl Absorb for ModelCounters {
    fn absorb(&mut self, other: &Self) {
        self.total.absorb(&other.total);
        self.usage_details.absorb(&other.usage_details);
        self.by_model.absorb(&other.by_model);
    }
}

impl Absorb for ToolUsage {
    fn absorb(&mut self, other: &Self) {
        self.calls.absorb(&other.calls);
        self.duration_ms.absorb(&other.duration_ms);
        self.bytes_sent.absorb(&other.bytes_sent);
        self.bytes_received.absorb(&other.bytes_received);
    }
}

impl Absorb for ToolCounters {
    fn absorb(&mut self, other: &Self) {
        self.calls.absorb(&other.calls);
        self.by_tool.absorb(&other.by_tool);
        self.mcp.absorb(&other.mcp);
    }
}

impl Absorb for FileCounters {
    fn absorb(&mut self, other: &Self) {
        self.events.absorb(&other.events);
        self.by_action.absorb(&other.by_action);
    }
}

impl Absorb for ExecCounters {
    fn absorb(&mut self, other: &Self) {
        self.started.absorb(&other.started);
        self.completed.absorb(&other.completed);
    }
}

/// The earliest of two fixed-width RFC 3339 stamps; empty means unset.
fn earliest(current: &mut String, other: &str) {
    if !other.is_empty() && (current.is_empty() || other < current.as_str()) {
        *current = other.to_string();
    }
}

fn latest(current: &mut String, other: &str) {
    if other > current.as_str() {
        *current = other.to_string();
    }
}

impl Absorb for ProcessUsage {
    fn absorb(&mut self, other: &Self) {
        self.count.absorb(&other.count);
        earliest(&mut self.first_seen, &other.first_seen);
        latest(&mut self.last_seen, &other.last_seen);
    }
}

impl Absorb for AuditCounters {
    fn absorb(&mut self, other: &Self) {
        self.events.absorb(&other.events);
        self.by_exe.absorb(&other.by_exe);
    }
}

impl Absorb for RuleUsage {
    fn absorb(&mut self, other: &Self) {
        self.count.absorb(&other.count);
        if other.latest_timestamp_unix_ms > self.latest_timestamp_unix_ms {
            self.latest_timestamp_unix_ms = other.latest_timestamp_unix_ms;
            self.latest_event_id = other.latest_event_id.clone();
        }
    }
}

impl Absorb for SecurityCounters {
    fn absorb(&mut self, other: &Self) {
        self.matches.absorb(&other.matches);
        self.by_action.absorb(&other.by_action);
        self.by_event_type.absorb(&other.by_event_type);
        self.by_level.absorb(&other.by_level);
        self.by_rule.absorb(&other.by_rule);
        self.open_asks_overflow.absorb(&other.open_asks_overflow);
        for ask in &other.open_asks {
            if self.open_asks.contains(ask) {
                continue;
            }
            if self.open_asks.len() < MAX_OPEN_ASKS {
                self.open_asks.insert(ask.clone());
            } else {
                self.open_asks_overflow.absorb(&1);
            }
        }
    }
}

impl Absorb for PluginCounters {
    fn absorb(&mut self, other: &Self) {
        self.executions.absorb(&other.executions);
        self.applied.absorb(&other.applied);
        self.skipped.absorb(&other.skipped);
        self.total_duration_us.absorb(&other.total_duration_us);
        self.max_duration_us = self.max_duration_us.max(other.max_duration_us);
        self.detections.absorb(&other.detections);
    }
}

impl Absorb for CredentialCounters {
    fn absorb(&mut self, other: &Self) {
        if other.provider > self.provider {
            self.provider = other.provider.clone();
        }
        self.substitutions.absorb(&other.substitutions);
        self.injected_substitutions.absorb(&other.injected_substitutions);
        self.observations.absorb(&other.observations);
        self.injections.absorb(&other.injections);
        self.last_seen_unix_ms = self.last_seen_unix_ms.max(other.last_seen_unix_ms);
    }
}

#[cfg(test)]
mod tests;
