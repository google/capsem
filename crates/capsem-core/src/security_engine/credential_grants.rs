//! Target preflight uses the real rule/plugin rail. It never authorizes a
//! fully facted request or replaces its runtime enforcement/materialization.
use super::*;
use crate::net::policy_config::ActivePolicyFile;
use capsem_credentials::GrantCapability;

struct Snapshot {
    rules: SecurityRuleSet,
    plugins: Arc<BTreeMap<String, SecurityPluginConfig>>,
}
impl Snapshot {
    fn new(policy: &ActivePolicyFile) -> Result<Self, String> {
        Ok(Self {
            rules: policy.compile_security_rule_set()?,
            plugins: Arc::new(policy.plugins.clone()),
        })
    }
    fn admits(&self, capability: &GrantCapability) -> bool {
        if !self
            .plugins
            .get("credential_broker")
            .is_some_and(|plugin| plugin.mode == SecurityPluginMode::Rewrite)
        {
            return false;
        }
        capability.targets().iter().all(|target| {
            let url = target.destination();
            let event = SecurityEvent::new(RuntimeSecurityEventType::HttpRequest)
                .with_http(HttpSecurityEvent {
                    host: url.host_str().map(str::to_owned),
                    method: Some(target.method().into()),
                    path: Some(url.path().into()),
                    query: url.query().map(str::to_owned),
                    ..Default::default()
                })
                .with_http_request(HttpRequestSecurityEvent::new(
                    url.host_str().unwrap_or_default(),
                    None,
                    http::HeaderMap::new(),
                    url.query().map(str::to_owned),
                ))
                .with_tcp(TcpSecurityEvent {
                    port: url.port_or_known_default().map(|port| port.to_string()),
                });
            evaluate_security_boundary(&self.rules, Arc::clone(&self.plugins), event)
                .is_ok_and(|evaluation| evaluation.enforcement.is_allowed())
        })
    }
}

/// Precomputed outside grant admission locks; complete capability identity
/// includes scopes and targets, since profile-local IDs can be reused.
pub struct CredentialGrantAdmission {
    allowed: BTreeMap<String, Vec<GrantCapability>>,
}
impl CredentialGrantAdmission {
    pub fn deny_all() -> Self {
        Self {
            allowed: BTreeMap::new(),
        }
    }
    pub fn new<'a>(
        current: &ActivePolicyFile,
        applied: &ActivePolicyFile,
        capabilities: impl Iterator<Item = &'a GrantCapability>,
    ) -> Result<Self, String> {
        let current = Snapshot::new(current)?;
        let applied = Snapshot::new(applied)?;
        let mut allowed: BTreeMap<String, Vec<GrantCapability>> = BTreeMap::new();
        for capability in capabilities {
            if current.admits(capability) && applied.admits(capability) {
                allowed
                    .entry(capability.id().into())
                    .or_default()
                    .push(capability.clone());
            }
        }
        Ok(Self { allowed })
    }
    pub fn admits(&self, capability: &GrantCapability) -> bool {
        self.allowed
            .get(capability.id())
            .is_some_and(|allowed| allowed.contains(capability))
    }
}
