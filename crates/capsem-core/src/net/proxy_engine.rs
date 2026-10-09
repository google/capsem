//! Transport-independent state for one session's HTTP and DNS proxy.
//!
//! A request clones one [`ProxyPolicySnapshot`] and keeps it through routing,
//! enforcement, credential handling, and ledger completion. Reload replaces
//! the outer `Arc` once, so no request can combine pieces from two active
//! policy files.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use super::policy::NetworkMechanics;
use super::policy_config::{ModelEndpointRegistry, SecurityPluginConfig, SecurityRuleSet};

/// One fully compiled active-policy revision.
pub struct ProxyPolicySnapshot {
    digest: String,
    network: NetworkMechanics,
    security_rules: Arc<SecurityRuleSet>,
    plugins: Arc<BTreeMap<String, SecurityPluginConfig>>,
    model_endpoints: ModelEndpointRegistry,
}

impl ProxyPolicySnapshot {
    pub fn new(
        digest: String,
        network: NetworkMechanics,
        security_rules: SecurityRuleSet,
        plugins: BTreeMap<String, SecurityPluginConfig>,
        model_endpoints: ModelEndpointRegistry,
    ) -> Self {
        Self {
            digest,
            network,
            security_rules: Arc::new(security_rules),
            plugins: Arc::new(plugins),
            model_endpoints,
        }
    }

    pub fn digest(&self) -> &str {
        &self.digest
    }

    pub fn network(&self) -> &NetworkMechanics {
        &self.network
    }

    pub fn security_rules(&self) -> &Arc<SecurityRuleSet> {
        &self.security_rules
    }

    pub fn plugins(&self) -> &Arc<BTreeMap<String, SecurityPluginConfig>> {
        &self.plugins
    }

    pub fn model_endpoints(&self) -> &ModelEndpointRegistry {
        &self.model_endpoints
    }
}

/// Live handle whose sole mutable operation atomically replaces a revision.
#[derive(Clone)]
pub struct ProxyPolicyHandle {
    current: Arc<RwLock<Arc<ProxyPolicySnapshot>>>,
}

impl ProxyPolicyHandle {
    pub fn new(snapshot: ProxyPolicySnapshot) -> Self {
        Self {
            current: Arc::new(RwLock::new(Arc::new(snapshot))),
        }
    }

    pub fn snapshot(&self) -> Arc<ProxyPolicySnapshot> {
        self.current.read().unwrap_or_else(|error| error.into_inner()).clone()
    }

    pub fn replace(&self, snapshot: ProxyPolicySnapshot) {
        *self.current.write().unwrap_or_else(|error| error.into_inner()) = Arc::new(snapshot);
    }
}

#[cfg(test)]
mod tests;
