//! Finite host-side ownership policy. No lease duration comes from a VM or SDK.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ManagedSessionPolicy {
    pub lease_seconds: u32,
    pub renew_after_seconds: u32,
    pub tombstone_seconds: u32,
}

impl Default for ManagedSessionPolicy {
    fn default() -> Self {
        Self {
            lease_seconds: 300,
            renew_after_seconds: 60,
            tombstone_seconds: 86_400,
        }
    }
}

impl ManagedSessionPolicy {
    pub fn validate(&self) -> Result<(), String> {
        if self.lease_seconds == 0
            || self.renew_after_seconds == 0
            || self.renew_after_seconds >= self.lease_seconds
            || self.tombstone_seconds == 0
        {
            return Err("managed session policy requires positive durations and renewal before lease expiry".into());
        }
        Ok(())
    }

    /// Corporate policy limits live ownership and extends replay protection.
    /// A local setting cannot lengthen the corporate lease or shorten its
    /// tombstones. Invalid intersections fail rather than disabling expiry.
    pub fn resolve(user: Option<&Self>, corp: Option<&Self>) -> Result<Self, String> {
        let mut policy = user.copied().unwrap_or_default();
        policy.validate()?;
        if let Some(corp) = corp {
            corp.validate()?;
            policy.lease_seconds = policy.lease_seconds.min(corp.lease_seconds);
            policy.renew_after_seconds = policy.renew_after_seconds.min(corp.renew_after_seconds);
            policy.tombstone_seconds = policy.tombstone_seconds.max(corp.tombstone_seconds);
        }
        policy.validate()?;
        Ok(policy)
    }
}
