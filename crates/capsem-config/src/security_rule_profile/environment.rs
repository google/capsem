//! Leaf compilation reuses the existing matcher. Core owns policy composition.
use super::*;
use crate::environment::{EnvironmentAccessRule, EnvironmentDetectionRule};
mod mcp_permissions;
pub use mcp_permissions::compile_environment_mcp_permissions;

impl CompiledSecurityRule {
    /// Own the bounded source and compiled AST once. Runtime evaluation neither
    /// reparses configuration nor copies an environment snapshot.
    pub fn from_environment_access(key: &str, rule: &EnvironmentAccessRule) -> Result<Self, String> {
        rule.validate_structure(key)?;
        let compiled_condition =
            compile_security_event_match(&rule.condition).map_err(|_| "invalid environment access rule condition")?;
        Ok(Self {
            rule_id: format!("environment.access.{key}"),
            provider: "environment".into(),
            namespace: "environment.access".into(),
            rule_key: key.into(),
            default_rule: false,
            enabled: rule.enabled,
            name: rule.name.clone(),
            action: rule.action,
            condition: rule.condition.clone(),
            compiled_condition,
            detection_level: None,
            priority: rule.priority.unwrap_or(USER_PRIORITY_MIN),
            corp_locked: false,
            reason: None,
            managed: None,
        })
    }
}

/// A detection observation has no access action or priority. It cannot be put
/// into a SecurityRuleSet or accidentally authorize a matching event.
#[derive(Clone, Debug)]
pub struct CompiledEnvironmentDetectionRule {
    pub rule_id: String,
    pub name: String,
    pub condition: String,
    pub level: DetectionLevel,
    pub enabled: bool,
    compiled_condition: CompiledCondition,
}
impl CompiledEnvironmentDetectionRule {
    pub fn new(key: &str, rule: &EnvironmentDetectionRule) -> Result<Self, String> {
        rule.validate_structure(key)?;
        let compiled_condition = compile_security_event_match(&rule.condition)
            .map_err(|_| "invalid environment detection rule condition")?;
        Ok(Self {
            rule_id: format!("environment.detection.{key}"),
            name: rule.name.clone(),
            condition: rule.condition.clone(),
            level: rule.level,
            enabled: rule.enabled,
            compiled_condition,
        })
    }
    pub fn evaluate<S: PolicySubject + ?Sized>(&self, subject: &S) -> Result<Option<DetectionLevel>, String> {
        if !self.enabled {
            return Ok(None);
        }
        Ok(self.compiled_condition.evaluate(subject)?.then_some(self.level))
    }
}
