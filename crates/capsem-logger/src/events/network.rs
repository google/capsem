//! Canonical named-network records: the network itself and each VM's membership.
//!
//! Validated at construction so a malformed record never reaches SQLite: the
//! registry that owns these is the sole authority, and a CHECK failure inside
//! a batched write would be a producer bug reported far from its cause.
use std::net::Ipv4Addr;
use uuid::Uuid;

pub const NETWORK_NAME_MAX: usize = 64;
pub const VM_ID_MAX: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkState {
    Active,
    Retired,
}

impl NetworkState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Retired => "retired",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MembershipState {
    Declared,
    Attaching,
    Ready,
    Failed,
    Detached,
}

impl MembershipState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Declared => "declared",
            Self::Attaching => "attaching",
            Self::Ready => "ready",
            Self::Failed => "failed",
            Self::Detached => "detached",
        }
    }
}

/// One named network. Written once at creation and again when retired;
/// the row is upserted by id, so a later write is a state change, never a
/// second network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkRecord {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) state: NetworkState,
    pub(crate) created_unix_ms: i64,
    pub(crate) retired_unix_ms: Option<i64>,
}

impl NetworkRecord {
    pub fn new(id: Uuid, name: &str, created_unix_ms: i64) -> Result<Self, String> {
        if id.is_nil() {
            return Err("network record requires a non-nil id".into());
        }
        if name.is_empty() || name.len() > NETWORK_NAME_MAX || name.chars().any(|c| c.is_whitespace() || c.is_control())
        {
            return Err(format!(
                "network name must be 1..={NETWORK_NAME_MAX} characters without whitespace or control characters"
            ));
        }
        if created_unix_ms < 0 {
            return Err("network creation time must not be negative".into());
        }
        Ok(Self {
            id: id.to_string(),
            name: name.to_string(),
            state: NetworkState::Active,
            created_unix_ms,
            retired_unix_ms: None,
        })
    }

    /// The same network, retired at `retired_unix_ms`.
    pub fn retired(mut self, retired_unix_ms: i64) -> Result<Self, String> {
        if retired_unix_ms < self.created_unix_ms {
            return Err("network cannot retire before it was created".into());
        }
        self.state = NetworkState::Retired;
        self.retired_unix_ms = Some(retired_unix_ms);
        Ok(self)
    }

    pub fn id(&self) -> &str {
        &self.id
    }
}

/// One VM's membership of one network, keyed by both. Written on every state
/// change; the row is upserted, so the table holds the current state of each
/// membership and the ledger holds its history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkMembership {
    pub(crate) network_id: String,
    pub(crate) vm_id: String,
    pub(crate) address: String,
    pub(crate) state: MembershipState,
    pub(crate) updated_unix_ms: i64,
}

impl NetworkMembership {
    pub fn new(
        network_id: Uuid,
        vm_id: &str,
        address: Ipv4Addr,
        state: MembershipState,
        updated_unix_ms: i64,
    ) -> Result<Self, String> {
        if network_id.is_nil() {
            return Err("membership requires a non-nil network id".into());
        }
        if vm_id.is_empty() || vm_id.len() > VM_ID_MAX || vm_id.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(format!(
                "membership vm id must be 1..={VM_ID_MAX} characters without whitespace"
            ));
        }
        if address.is_unspecified() || address.is_loopback() || address.is_broadcast() {
            return Err(format!(
                "membership address {address} is not a routable private address"
            ));
        }
        if updated_unix_ms < 0 {
            return Err("membership update time must not be negative".into());
        }
        Ok(Self {
            network_id: network_id.to_string(),
            vm_id: vm_id.to_string(),
            address: address.to_string(),
            state,
            updated_unix_ms,
        })
    }

    pub fn state(&self) -> MembershipState {
        self.state
    }
}
