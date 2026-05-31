use std::path::Path;
use std::sync::Arc;

use capsem_core::mcp::policy::{McpPolicy, McpUserConfig};
use capsem_core::net::mitm_proxy::RuntimeSecurityEngine;
use capsem_core::settings_profiles;
use capsem_core::vm::guest_config::GuestConfig;
use capsem_network_engine::domain_policy::{Action, DomainPolicy};
use tracing::warn;

mod guest_config;
mod match_recorder;
mod mcp_config;
mod rules;

pub(crate) use match_recorder::RuntimeRuleMatchAccumulator;

const DEFAULT_SNAPSHOT_AUTO_MAX: usize = 10;
const DEFAULT_SNAPSHOT_MANUAL_MAX: usize = 12;
const DEFAULT_SNAPSHOT_INTERVAL_SECS: u64 = 300;

#[derive(Clone)]
pub(crate) struct SecurityRuntimeState {
    pub(crate) profile_id: String,
    pub(crate) guest_config: GuestConfig,
    pub(crate) domain_policy: DomainPolicy,
    pub(crate) security_engine: Option<Arc<dyn RuntimeSecurityEngine>>,
    pub(crate) mcp_policy: McpPolicy,
    pub(crate) mcp_user: McpUserConfig,
    pub(crate) mcp_corp: McpUserConfig,
    pub(crate) snapshot_auto_max: usize,
    pub(crate) snapshot_manual_max: usize,
    pub(crate) snapshot_interval_secs: u64,
}

#[cfg(test)]
pub(crate) fn load_security_runtime_state(session_dir: &Path) -> SecurityRuntimeState {
    load_security_runtime_state_with_runtime_rules(session_dir, None)
}

#[cfg(test)]
pub(crate) fn load_security_runtime_state_with_runtime_rules(
    session_dir: &Path,
    runtime_rules: Option<&capsem_proto::ipc::RuntimeSecurityRulesSnapshot>,
) -> SecurityRuntimeState {
    load_security_runtime_state_with_runtime_rules_and_recorder(session_dir, runtime_rules, None)
}

pub(crate) fn load_security_runtime_state_with_runtime_rules_and_recorder(
    session_dir: &Path,
    runtime_rules: Option<&capsem_proto::ipc::RuntimeSecurityRulesSnapshot>,
    match_recorder: Option<RuntimeRuleMatchAccumulator>,
) -> SecurityRuntimeState {
    load_security_runtime_state_from_effective_with_runtime_rules(
        session_dir,
        runtime_rules,
        match_recorder,
    )
}

#[cfg(test)]
pub(crate) fn load_security_runtime_state_from_effective(
    session_dir: &Path,
) -> SecurityRuntimeState {
    load_security_runtime_state_from_effective_with_runtime_rules(session_dir, None, None)
}

fn load_security_runtime_state_from_effective_with_runtime_rules(
    session_dir: &Path,
    runtime_rules: Option<&capsem_proto::ipc::RuntimeSecurityRulesSnapshot>,
    match_recorder: Option<RuntimeRuleMatchAccumulator>,
) -> SecurityRuntimeState {
    let effective = load_effective_vm_settings_with_fallback(session_dir);

    let domain_policy = DomainPolicy::new(&[], &[], Action::Allow);
    let mut enforcement_rules = Vec::new();
    let mut detection_rules = Vec::new();
    if let Some(runtime_rules) = runtime_rules {
        enforcement_rules.extend(
            runtime_rules
                .enforcement
                .iter()
                .cloned()
                .map(rules::cel_enforcement_rule_from_snapshot),
        );
        detection_rules.extend(
            runtime_rules
                .detection
                .iter()
                .cloned()
                .map(rules::cel_detection_rule_from_snapshot),
        );
    }
    enforcement_rules.extend(
        effective
            .as_ref()
            .map(rules::runtime_enforcement_rules_from_effective)
            .unwrap_or_default(),
    );
    let security_engine = rules::build_runtime_security_engine_from_rules(
        effective.as_ref(),
        enforcement_rules,
        detection_rules,
        match_recorder,
    );

    let mcp_user = effective
        .as_ref()
        .map(mcp_config::mcp_user_config_from_effective)
        .unwrap_or_default();
    let mcp_corp = McpUserConfig::default();
    let mcp_policy = mcp_user.to_policy(&mcp_corp);
    let guest_config = guest_config::guest_config_from_effective(effective.as_ref());
    let profile_id = effective
        .as_ref()
        .map(|effective| effective.profile_id.clone())
        .unwrap_or_else(|| "unknown".to_string());

    SecurityRuntimeState {
        profile_id,
        guest_config,
        domain_policy,
        security_engine,
        mcp_policy,
        mcp_user,
        mcp_corp,
        snapshot_auto_max: DEFAULT_SNAPSHOT_AUTO_MAX,
        snapshot_manual_max: DEFAULT_SNAPSHOT_MANUAL_MAX,
        snapshot_interval_secs: DEFAULT_SNAPSHOT_INTERVAL_SECS,
    }
}

fn load_effective_vm_settings_with_fallback(
    session_dir: &Path,
) -> Option<settings_profiles::EffectiveVmSettings> {
    match settings_profiles::load_vm_effective_settings(session_dir) {
        Ok(effective) => Some(effective),
        Err(error) => {
            warn!(
                error = %error,
                session_dir = %session_dir.display(),
                "failed to load vm-effective settings attachment; falling back to default profile"
            );
            let defaults = settings_profiles::ProfileRootSettings::default();
            match settings_profiles::resolve_effective_vm_settings(&defaults, None) {
                Ok(effective) => Some(effective),
                Err(resolve_error) => {
                    warn!(
                        error = %resolve_error,
                        "failed to resolve fallback default profile; running with open runtime policies"
                    );
                    None
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
