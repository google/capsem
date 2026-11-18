//! Selected rule/plugin inputs for the existing service-owned active-policy file.
//! Revision and generation identities belong to the instance, not these bytes.
use super::{EnvironmentPolicyInputs, RuleCatalogInputs};
use capsem_config::{
    catalogs::{ResolvedCatalogRule, RuleCatalogDefinition},
    environment::*,
    plugins::EnvironmentPlugins,
    SecurityRuleSet,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentRuntimePolicy {
    access: EnvironmentAccessRules,
    detection: EnvironmentDetectionRules,
    plugins: EnvironmentPlugins,
    corporate_plugins: EnvironmentPlugins,
    mcp_servers: Vec<String>,
    mcp_permissions: BTreeMap<String, EnvironmentMcpPermissions>,
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
            mcp_servers: inputs
                .mcp_servers()
                .keys()
                .cloned()
                .chain(inputs.mcp_catalog().selected.iter().map(|pin| pin.id.clone()))
                .collect(),
            mcp_permissions: inputs.mcp_permissions().clone(),
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
        Ok(())
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
