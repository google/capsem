//! Compile acknowledged environment sections through the existing policy rail.
use super::EnvironmentPolicyInputs;
use crate::net::policy_config::ActivePolicyFile;
use capsem_config::{
    environment::EnvironmentRules, CompiledEnvironmentDetectionRule, CompiledSecurityRule, SecurityRuleSet,
    SettingsFile,
};

fn require_rule_content<C, R>(section: &EnvironmentRules<C, R>) -> Result<(), String> {
    if !section.catalog.selected.is_empty() {
        return Err("selected environment rule catalog content is unavailable".into());
    }
    if !section.parameters.is_empty() {
        return Err("environment rule parameters require resolved catalog content".into());
    }
    if section.custom.len() > capsem_proto::ENVIRONMENT_MAX_RULES {
        return Err("too many environment custom rules".into());
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
    /// Until verified catalog definitions are supplied by their owning resolver,
    /// selected catalog rules fail explicitly instead of disappearing from the
    /// effective rule set. No ambient user settings are consulted.
    pub fn compile_rules(&self) -> Result<SecurityRuleSet, String> {
        require_rule_content(self.access_rules())?;
        require_rule_content(self.detection_rules())?;
        let baseline = ActivePolicyFile::from_settings_and_corp(&SettingsFile::default(), self.corporate())?
            .compile_security_rule_set()?;
        let mut rules = baseline.rules().to_vec();
        for (id, rule) in &self.access_rules().custom {
            rules.push(CompiledSecurityRule::from_environment_access(id, rule)?);
        }
        rules.extend(self.compile_mcp_permissions()?);
        let observations = self
            .detection_rules()
            .custom
            .iter()
            .map(|(id, rule)| CompiledEnvironmentDetectionRule::new(id, rule))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(SecurityRuleSet::with_observations(rules, observations))
    }
}
