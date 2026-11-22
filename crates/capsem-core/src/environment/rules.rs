//! Compile acknowledged environment sections through the existing policy rail.
use super::{package_registries, EnvironmentPolicyInputs, RuleCatalogInputs};
use crate::net::policy_config::ActivePolicyFile;
use capsem_config::{
    catalogs::ResolvedCatalogRule,
    environment::{EnvironmentAccessRules, EnvironmentCatalogPin, EnvironmentDetectionRules, EnvironmentRuleParameter},
    CompiledEnvironmentDetectionRule, CompiledSecurityRule, SecurityRuleSet,
};

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
        let baseline = ActivePolicyFile::compile_corporate_rule_set(self.corporate())?;
        compile_selected_rules(
            self.access_rules(),
            self.detection_rules(),
            self.compile_mcp_permissions()?,
            package_registries::selected(self.package_access()),
            baseline,
            |pin, values| {
                catalogs
                    .ok_or("selected environment rule catalog content is unavailable")?
                    .resolve(pin, values)
            },
        )
    }
}

pub(super) fn compile_selected_rules(
    access: &EnvironmentAccessRules,
    detection: &EnvironmentDetectionRules,
    mcp_rules: Vec<CompiledSecurityRule>,
    registries: &[String],
    baseline: SecurityRuleSet,
    mut resolve: impl FnMut(
        &EnvironmentCatalogPin,
        &std::collections::BTreeMap<String, EnvironmentRuleParameter>,
    ) -> Result<ResolvedCatalogRule, String>,
) -> Result<SecurityRuleSet, String> {
    access.validate()?;
    detection.validate()?;
    let mut rules = baseline.rules().to_vec();
    for (id, rule) in &access.custom {
        rules.push(CompiledSecurityRule::from_environment_access(id, rule)?);
    }
    let empty = std::collections::BTreeMap::new();
    for pin in &access.catalog.selected {
        let values = access.parameters.get(&pin.id).unwrap_or(&empty);
        let ResolvedCatalogRule::Access(rule) = resolve(pin, values)? else {
            return Err("selected catalog rule is not an access rule".into());
        };
        let mut rule = CompiledSecurityRule::from_environment_access(&pin.id, &rule)?;
        rule.rule_id = format!("environment.access.catalog.{}", pin.id);
        rule.namespace = "environment.access.catalog".into();
        rules.push(rule);
    }
    rules.extend(mcp_rules);
    rules.extend(package_registries::compile(registries)?);
    let mut observations = detection
        .custom
        .iter()
        .map(|(id, rule)| CompiledEnvironmentDetectionRule::new(id, rule))
        .collect::<Result<Vec<_>, _>>()?;
    for pin in &detection.catalog.selected {
        let values = detection.parameters.get(&pin.id).unwrap_or(&empty);
        let ResolvedCatalogRule::Detection(rule) = resolve(pin, values)? else {
            return Err("selected catalog rule is not a detection rule".into());
        };
        let mut observation = CompiledEnvironmentDetectionRule::new(&pin.id, &rule)?;
        observation.rule_id = format!("environment.detection.catalog.{}", pin.id);
        observations.push(observation);
    }
    Ok(SecurityRuleSet::with_observations(rules, observations))
}
