//! Identities of the one in-flight descriptor handoff.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum AwaitingAdoption {
    Upstream(u64),
    Ledger(u64),
    Listener(u64),
    Aggregator(u64),
}

impl AwaitingAdoption {
    pub(super) const fn grant_id(self) -> u64 {
        match self {
            Self::Upstream(grant_id)
            | Self::Ledger(grant_id)
            | Self::Listener(grant_id)
            | Self::Aggregator(grant_id) => grant_id,
        }
    }
}
