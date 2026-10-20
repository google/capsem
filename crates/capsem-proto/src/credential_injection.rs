//! Host-only material for service-to-owner handoff; never guest configuration.

use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialMaterial {
    pub provider: String,
    pub credential_ref: String,
    pub value: String,
}

impl std::fmt::Debug for CredentialMaterial {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CredentialMaterial").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests;
