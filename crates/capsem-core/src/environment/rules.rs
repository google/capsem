//! Compile acknowledged environment sections through the existing policy rail.
use super::{EnvironmentPolicyInputs, RuleCatalogInputs};
use crate::net::policy_config::ActivePolicyFile;
use capsem_config::{
    catalogs::ResolvedCatalogRule,
    environment::{EnvironmentCatalogPin, EnvironmentRules},
    CompiledEnvironmentDetectionRule, CompiledSecurityRule, SecurityRuleSet, SettingsFile,
};

fn validate_section<R>(section: &EnvironmentRules<EnvironmentCatalogPin, R>) -> Result<(), String> {
    if section.custom.len() > capsem_proto::ENVIRONMENT_MAX_RULES
        || section.catalog.selected.len() > capsem_proto::ENVIRONMENT_MAX_CATALOG_SELECTIONS
        || section.parameters.len() > capsem_proto::ENVIRONMENT_MAX_CATALOG_SELECTIONS
    {
        return Err("too many environment rules or catalog selections".into());
    }
    let mut selected = std::collections::BTreeSet::new();
    for pin in &section.catalog.selected {
        if !selected.insert(pin.id.as_str()) || section.custom.contains_key(&pin.id) {
            return Err("duplicate environment custom or catalog rule identity".into());
        }
    }
    for (key, values) in &section.parameters {
        if !selected.contains(key.as_str()) || values.len() > capsem_proto::ENVIRONMENT_MAX_CONFIGURATION_ENTRIES {
            return Err("environment rule parameters require bounded selected catalog entries".into());
        }
    }
    Ok(())
}

impl EnvironmentPolicyInputs<'_> {
    /// Access and MCP permissions follow the acknowledged live revision;
    /// detections follow the instantiated boot revision. Corporate constraints
    /// always come from the separately supplied current configuration.
    ///
    /// This compiles rules only: it neither approves nor publishes a policy and
    /// does not replace MCP definition/pin, credential or server admission gates.
    /// Selected catalogs require explicit verified content; they never disappear
    /// from the effective set on failure. No ambient settings are consulted.
    pub fn compile_rules(&self) -> Result<SecurityRuleSet, String> {
        self.compile_rules_using(None)
    }

    pub fn compile_rules_with_catalogs(&self, catalogs: &RuleCatalogInputs<'_>) -> Result<SecurityRuleSet, String> {
        self.compile_rules_using(Some(catalogs))
    }

    fn compile_rules_using(&self, catalogs: Option<&RuleCatalogInputs<'_>>) -> Result<SecurityRuleSet, String> {
        validate_section(self.access_rules())?;
        validate_section(self.detection_rules())?;
        let baseline = ActivePolicyFile::from_settings_and_corp(&SettingsFile::default(), self.corporate())?
            .compile_security_rule_set()?;
        let mut rules = baseline.rules().to_vec();
        for (id, rule) in &self.access_rules().custom {
            rules.push(CompiledSecurityRule::from_environment_access(id, rule)?);
        }
        let empty = std::collections::BTreeMap::new();
        for pin in &self.access_rules().catalog.selected {
            let catalog = catalogs.ok_or("selected environment rule catalog content is unavailable")?;
            let values = self.access_rules().parameters.get(&pin.id).unwrap_or(&empty);
            let ResolvedCatalogRule::Access(rule) = catalog.resolve(pin, values)? else {
                return Err("selected catalog rule is not an access rule".into());
            };
            let mut rule = CompiledSecurityRule::from_environment_access(&pin.id, &rule)?;
            rule.rule_id = format!("environment.access.catalog.{}", pin.id);
            rule.namespace = "environment.access.catalog".into();
            rules.push(rule);
        }
        rules.extend(self.compile_mcp_permissions()?);
        let mut observations = self
            .detection_rules()
            .custom
            .iter()
            .map(|(id, rule)| CompiledEnvironmentDetectionRule::new(id, rule))
            .collect::<Result<Vec<_>, _>>()?;
        for pin in &self.detection_rules().catalog.selected {
            let catalog = catalogs.ok_or("selected environment rule catalog content is unavailable")?;
            let values = self.detection_rules().parameters.get(&pin.id).unwrap_or(&empty);
            let ResolvedCatalogRule::Detection(rule) = catalog.resolve(pin, values)? else {
                return Err("selected catalog rule is not a detection rule".into());
            };
            let mut observation = CompiledEnvironmentDetectionRule::new(&pin.id, &rule)?;
            observation.rule_id = format!("environment.detection.catalog.{}", pin.id);
            observations.push(observation);
        }
        Ok(SecurityRuleSet::with_observations(rules, observations))
    }
}
