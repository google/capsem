//! Transport-independent state for one session's HTTP and DNS proxy.
//!
//! A request clones one [`ProxyPolicySnapshot`] and keeps it through routing,
//! enforcement, credential handling, and ledger completion. Reload replaces
//! the outer `Arc` once, so no request can combine pieces from two active
//! policy files.
//!
//! Transport adapters pair this engine with the descriptor-only
//! [`super::mitm_proxy::TcpUpstreamGrants`], [`super::dns::DnsUpstreamGrants`]
//! and [`super::dns::private::PrivateNames`] capabilities. None exposes a
//! filesystem path, process handle, resolver socket, or arbitrary dial.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, RwLock};

use capsem_logger::{DbWriter, WriteOp};

use crate::credential_broker::{BrokeredCredential, BrokeredUpstreamCredentials, CredentialObservation};
use crate::net::ai_traffic::provider::ProviderKind;
use crate::security_engine::{MaterializedHttpRequest, SecurityActionError, SecurityBoundaryEvaluation, SecurityEvent};

use super::policy::NetworkMechanics;
use super::policy_config::{ModelEndpointRegistry, SecurityPluginConfig, SecurityRuleSet};

pub type ProxyCapabilityFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The only ledger authority exposed to a proxy worker.
pub trait ProxyLedger: Send + Sync {
    fn write(&self, op: WriteOp) -> ProxyCapabilityFuture<'_, Result<(), String>>;
}

impl ProxyLedger for DbWriter {
    fn write(&self, op: WriteOp) -> ProxyCapabilityFuture<'_, Result<(), String>> {
        Box::pin(self.write_checked(op))
    }
}

/// Credential operations available to a proxy worker, without store paths.
pub trait ProxyCredentials: Send + Sync {
    fn capture(&self, observation: &CredentialObservation) -> Result<BrokeredCredential, String>;

    fn substitute_upstream(
        &self,
        domain: &str,
        ai_provider: Option<ProviderKind>,
        headers: &mut http::HeaderMap,
        query: Option<&str>,
    ) -> Result<BrokeredUpstreamCredentials, String>;

    fn redact_text(&self, text: &str, observations: &[CredentialObservation]) -> String {
        crate::credential_broker::redact_observed_credentials_in_text(text, observations)
    }

    fn redact_bytes(&self, bytes: &[u8], observations: &[CredentialObservation]) -> Vec<u8> {
        crate::credential_broker::redact_observed_credentials_in_bytes(bytes, observations)
    }
}

/// In-process adapter used until the proxy runs behind the coordinator IPC.
pub struct LocalProxyCredentials;

impl ProxyCredentials for LocalProxyCredentials {
    fn capture(&self, observation: &CredentialObservation) -> Result<BrokeredCredential, String> {
        crate::credential_broker::broker_observed_credential(observation)
    }

    fn substitute_upstream(
        &self,
        domain: &str,
        ai_provider: Option<ProviderKind>,
        headers: &mut http::HeaderMap,
        query: Option<&str>,
    ) -> Result<BrokeredUpstreamCredentials, String> {
        crate::credential_broker::substitute_brokered_upstream_credentials(domain, ai_provider, headers, query)
    }
}

/// Transport-independent proxy behavior and its scoped host capabilities.
#[derive(Clone)]
pub struct ProxyEngine {
    policy: ProxyPolicyHandle,
    ledger: Arc<dyn ProxyLedger>,
    credentials: Arc<dyn ProxyCredentials>,
}

impl ProxyEngine {
    pub fn new(
        policy: ProxyPolicyHandle,
        ledger: Arc<dyn ProxyLedger>,
        credentials: Arc<dyn ProxyCredentials>,
    ) -> Self {
        Self {
            policy,
            ledger,
            credentials,
        }
    }

    pub fn local(policy: ProxyPolicyHandle, ledger: Arc<DbWriter>) -> Self {
        Self::new(policy, ledger, Arc::new(LocalProxyCredentials))
    }

    pub fn policy(&self) -> &ProxyPolicyHandle {
        &self.policy
    }

    pub async fn write(&self, op: WriteOp) -> Result<(), String> {
        self.ledger.write(op).await
    }

    pub fn evaluate(
        &self,
        snapshot: &ProxyPolicySnapshot,
        event: SecurityEvent,
    ) -> Result<SecurityBoundaryEvaluation, SecurityActionError> {
        crate::security_engine::evaluate_security_boundary_with_credentials(
            snapshot.security_rules(),
            Arc::clone(snapshot.plugins()),
            Arc::clone(&self.credentials),
            event,
        )
    }

    pub fn capture_credential(&self, observation: &CredentialObservation) -> Result<BrokeredCredential, String> {
        self.credentials.capture(observation)
    }

    pub fn credentials(&self) -> Arc<dyn ProxyCredentials> {
        Arc::clone(&self.credentials)
    }

    pub fn materialize_http_request(
        &self,
        event: &SecurityEvent,
    ) -> Result<MaterializedHttpRequest, SecurityActionError> {
        crate::security_engine::materialize_http_request_for_upstream_with_credentials(event, self.credentials.as_ref())
    }

    pub fn redact_text(&self, text: &str, observations: &[CredentialObservation]) -> String {
        self.credentials.redact_text(text, observations)
    }

    pub fn redact_bytes(&self, bytes: &[u8], observations: &[CredentialObservation]) -> Vec<u8> {
        self.credentials.redact_bytes(bytes, observations)
    }
}

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
