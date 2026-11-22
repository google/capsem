//! Selected rule/plugin inputs for the existing service-owned active-policy file.
//! Revision and generation identities belong to the instance, not these bytes.
use super::{mcp_launch, package_registries, EnvironmentPolicyInputs, RuleCatalogInputs};
use capsem_config::{
    catalogs::{ResolvedCatalogRule, RuleCatalogDefinition},
    environment::*,
    mcp::McpConfig,
    plugins::EnvironmentPlugins,
    SecurityRuleSet,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentRuntimePolicy {
    access: EnvironmentAccessRules,
    detection: EnvironmentDetectionRules,
    plugins: EnvironmentPlugins,
    corporate_plugins: EnvironmentPlugins,
    mcp_servers: Vec<String>,
    /// Exact endpoint and ordinary headers from the acknowledged boot revision.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    mcp_remote: BTreeMap<String, EnvironmentMcpDefinition>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    mcp_sandbox: BTreeMap<String, EnvironmentMcpDefinition>,
    mcp_permissions: BTreeMap<String, EnvironmentMcpPermissions>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    package_registries: Vec<String>,
    /// Only the selected, resolved definitions, never entire catalog payloads.
    /// Original pin and parameter values remain correlated after serialization.
    catalog_rules: Vec<FrozenCatalogRule>,
}
impl std::fmt::Debug for EnvironmentRuntimePolicy {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("EnvironmentRuntimePolicy(..)")
    }
}
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FrozenCatalogRule {
    pin: EnvironmentCatalogPin,
    parameters: BTreeMap<String, EnvironmentRuleParameter>,
    definition: RuleCatalogDefinition,
}
impl EnvironmentRuntimePolicy {
    pub(crate) fn select(
        inputs: &EnvironmentPolicyInputs<'_>,
        catalogs: Option<&RuleCatalogInputs<'_>>,
    ) -> Result<Self, String> {
        inputs.access_rules().validate()?;
        inputs.detection_rules().validate()?;
        // Reject ambiguous legacy corporate modes before selecting typed options.
        inputs.compile_plugins()?;
        inputs.compile_mcp_permissions()?;
        mcp_launch::validate_environment_mcp_launch(inputs.boot_mcps(), inputs.has_workload())?;
        let (mcp_remote, mcp_sandbox): (BTreeMap<_, _>, BTreeMap<_, _>) = mcp_launch::definitions(inputs.boot_mcps())?
            .map(|(name, definition)| (name.clone(), definition.clone()))
            .partition(|(_, definition)| matches!(definition, EnvironmentMcpDefinition::Remote { .. }));
        let mut catalog_rules = Vec::new();
        for (pins, parameters) in [
            (
                &inputs.access_rules().catalog.selected,
                &inputs.access_rules().parameters,
            ),
            (
                &inputs.detection_rules().catalog.selected,
                &inputs.detection_rules().parameters,
            ),
        ] {
            for pin in pins {
                let empty = BTreeMap::new();
                let values = parameters.get(&pin.id).unwrap_or(&empty);
                let resolved = catalogs
                    .ok_or("selected environment rule catalog content is unavailable")?
                    .resolve(pin, values)?;
                catalog_rules.push(FrozenCatalogRule {
                    pin: pin.clone(),
                    parameters: values.clone(),
                    definition: match resolved {
                        ResolvedCatalogRule::Access(rule) => RuleCatalogDefinition::Access { rule },
                        ResolvedCatalogRule::Detection(rule) => RuleCatalogDefinition::Detection { rule },
                    },
                });
            }
        }
        let selected = Self {
            access: inputs.access_rules().clone(),
            detection: inputs.detection_rules().clone(),
            plugins: inputs.plugins().clone(),
            corporate_plugins: inputs.corporate().environment_plugins.clone().unwrap_or_default(),
            mcp_servers: mcp_remote
                .keys()
                .chain(mcp_sandbox.keys())
                .cloned()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
            mcp_remote,
            mcp_sandbox,
            mcp_permissions: inputs.mcp_permissions().clone(),
            package_registries: package_registries::selected(inputs.package_access()).to_vec(),
            catalog_rules,
        };
        selected.validate()?;
        Ok(selected)
    }
    pub(crate) fn validate(&self) -> Result<(), String> {
        self.access.validate()?;
        self.detection.validate()?;
        self.plugins.validate().map_err(String::from)?;
        self.corporate_plugins.validate().map_err(String::from)?;
        if self.catalog_rules.len() != self.access.catalog.selected.len() + self.detection.catalog.selected.len() {
            return Err("environment policy requires exactly its selected resolved catalog rules".into());
        }
        // Every selection must match exactly one retained pin/parameter pair.
        for (pins, parameters) in [
            (&self.access.catalog.selected, &self.access.parameters),
            (&self.detection.catalog.selected, &self.detection.parameters),
        ] {
            for pin in pins {
                let empty = BTreeMap::new();
                self.resolve(pin, parameters.get(&pin.id).unwrap_or(&empty))?;
            }
        }
        super::mcp_permissions::compile_selected_mcp_permissions(
            self.mcp_servers.iter().map(String::as_str),
            &self.mcp_permissions,
        )?;
        if self.mcp_remote.len() + self.mcp_sandbox.len() != self.mcp_servers.len()
            || self
                .mcp_servers
                .iter()
                .any(|id| self.mcp_remote.contains_key(id) == self.mcp_sandbox.contains_key(id))
        {
            return Err("environment policy requires each approved MCP endpoint".into());
        }
        for (name, definition) in &self.mcp_remote {
            if !matches!(definition, EnvironmentMcpDefinition::Remote { .. }) {
                return Err("invalid environment MCP remote definition".into());
            }
            mcp_launch::validate(name, definition, false)?;
        }
        for (name, definition) in &self.mcp_sandbox {
            if !matches!(definition, EnvironmentMcpDefinition::Sandbox { .. }) {
                return Err("invalid environment MCP sandbox definition".into());
            }
            mcp_launch::validate(name, definition, true)?;
        }
        Ok(())
    }

    pub(crate) fn compile_mcp(&self, corporate: Option<&McpConfig>) -> Result<(McpConfig, BTreeSet<String>), String> {
        let config = mcp_launch::compose(&self.mcp_remote, &self.mcp_sandbox, &self.mcp_permissions, corporate)?;
        Ok((config, self.mcp_remote.keys().cloned().collect()))
    }
    pub(crate) fn sandbox_mcps(&self, mcp: &McpConfig) -> Result<Vec<super::SandboxMcp>, String> {
        self.mcp_sandbox
            .iter()
            .map(|(name, definition)| {
                super::SandboxMcp::new(name, definition, mcp.server_enabled.get(name).copied().unwrap_or(true))
            })
            .collect()
    }
    fn resolve(
        &self,
        pin: &EnvironmentCatalogPin,
        parameters: &BTreeMap<String, EnvironmentRuleParameter>,
    ) -> Result<ResolvedCatalogRule, String> {
        let mut matching = self
            .catalog_rules
            .iter()
            .filter(|rule| &rule.pin == pin && &rule.parameters == parameters);
        let rule = matching.next().ok_or("selected resolved catalog rule is unavailable")?;
        if matching.next().is_some() {
            return Err("ambiguous selected resolved catalog rule".into());
        }
        Ok(match &rule.definition {
            RuleCatalogDefinition::Access { rule } => ResolvedCatalogRule::Access(rule.clone()),
            RuleCatalogDefinition::Detection { rule } => ResolvedCatalogRule::Detection(rule.clone()),
        })
    }
    pub(crate) fn compile_rules(&self, baseline: SecurityRuleSet) -> Result<SecurityRuleSet, String> {
        super::rules::compile_selected_rules(
            &self.access,
            &self.detection,
            super::mcp_permissions::compile_selected_mcp_permissions(
                self.mcp_servers.iter().map(String::as_str),
                &self.mcp_permissions,
            )?,
            &self.package_registries,
            baseline,
            |pin, values| self.resolve(pin, values),
        )
    }
    pub(crate) fn compile_plugins(&self) -> Result<crate::security_engine::SecurityPluginPolicy, String> {
        Ok(std::sync::Arc::new(super::CompiledEnvironmentPlugins::resolve(
            &self.plugins,
            &self.corporate_plugins,
        )?)
        .into())
    }
}
