//! Price coverage is recorded with committed usage, never inferred from a zero.
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct CostCoverage {
    pub priced_calls: u64,
    pub unpriced_calls: u64,
    /// Historical calls whose snapshots predate explicit price coverage facts.
    pub unknown_calls: u64,
}

impl From<&capsem_proto::ledger_counters::ModelUsage> for CostCoverage {
    fn from(usage: &capsem_proto::ledger_counters::ModelUsage) -> Self {
        Self {
            priced_calls: usage.priced_calls,
            unpriced_calls: usage.unpriced_calls,
            unknown_calls: usage
                .calls
                .saturating_sub(usage.priced_calls.saturating_add(usage.unpriced_calls)),
        }
    }
}

#[cfg(test)]
mod tests;
