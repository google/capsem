//! Environment rule intent reuses the existing condition language. Access
//! decisions and detection observations remain separate types and sections.
use super::*;
use crate::security_rule_profile::{DetectionLevel, SecurityRuleAction, USER_PRIORITY_MAX, USER_PRIORITY_MIN};

#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, bound(deserialize = "C: Deserialize<'de>, R: Deserialize<'de>"))]
pub struct EnvironmentRules<C, R> {
    #[serde(default)]
    pub catalog: EnvironmentCatalogSelections<C>,
    #[serde(default)]
    pub parameters: BTreeMap<String, BTreeMap<String, EnvironmentRuleParameter>>,
    #[serde(default)]
    pub custom: BTreeMap<String, R>,
}
impl<C, R> Default for EnvironmentRules<C, R> {
    fn default() -> Self {
        Self {
            catalog: Default::default(),
            parameters: BTreeMap::new(),
            custom: BTreeMap::new(),
        }
    }
}
pub type EnvironmentAccessRules<C = EnvironmentCatalogPin> = EnvironmentRules<C, EnvironmentAccessRule>;
pub type EnvironmentDetectionRules<C = EnvironmentCatalogPin> = EnvironmentRules<C, EnvironmentDetectionRule>;

/// Parameter kinds supported by the initial rule catalogs. The registry's
/// typed descriptor additionally validates names, choices and numeric bounds.
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum EnvironmentRuleParameter {
    Boolean(bool),
    Integer(i64),
    Text(String),
    Choices(Vec<String>),
}

#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentAccessRule {
    pub name: String,
    // Environment access accepts capability decisions, not the internal
    // preprocessing/rewriting action variants of SecurityRuleAction.
    #[cfg_attr(feature = "schema", schema(value_type = EnvironmentCapabilityDecision))]
    pub action: SecurityRuleAction,
    #[serde(rename = "match")]
    pub condition: String,
    #[serde(default = "enabled")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<i32>,
}

#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentDetectionRule {
    pub name: String,
    #[serde(rename = "match")]
    pub condition: String,
    pub level: DetectionLevel,
    #[serde(default = "enabled")]
    pub enabled: bool,
}
fn enabled() -> bool {
    true
}

fn text(value: &str, limit: usize) -> bool {
    value.len() <= limit
        && !value
            .chars()
            .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t'))
}
fn definition(id: &str, name: &str, condition: &str) -> Result<(), String> {
    if crate::validate_identifier_shape(id).is_err()
        || name.trim().is_empty()
        || !text(name, capsem_proto::ENVIRONMENT_MAX_NAME_BYTES)
        || !text(condition, capsem_proto::ENVIRONMENT_MAX_RULE_BYTES)
    {
        return Err("invalid environment custom rule".into());
    }
    Ok(())
}
fn parameters<'a, C, R>(
    section: &EnvironmentRules<C, R>,
    selected: impl Iterator<Item = &'a str>,
) -> Result<(), String> {
    use capsem_proto::{
        ENVIRONMENT_MAX_CONFIGURATION_ENTRIES as MAX_ENTRIES, ENVIRONMENT_MAX_CONFIGURATION_TEXT_BYTES as MAX_TEXT,
    };
    let selected: std::collections::BTreeSet<_> = selected.collect();
    if section.custom.len() > capsem_proto::ENVIRONMENT_MAX_RULES
        || section.parameters.len() > capsem_proto::ENVIRONMENT_MAX_CATALOG_SELECTIONS
        || section.custom.keys().any(|key| selected.contains(key.as_str()))
    {
        return Err("duplicate or too many environment rules".into());
    }
    for (id, values) in &section.parameters {
        if !selected.contains(id.as_str()) || values.len() > MAX_ENTRIES {
            return Err("environment rule parameters require a selected catalog entry".into());
        }
        for (key, value) in values {
            let valid = match value {
                EnvironmentRuleParameter::Boolean(_) | EnvironmentRuleParameter::Integer(_) => true,
                EnvironmentRuleParameter::Text(value) => text(value, MAX_TEXT),
                EnvironmentRuleParameter::Choices(values) => {
                    values.len() <= MAX_ENTRIES && values.iter().all(|value| text(value, MAX_TEXT))
                }
            };
            if crate::validate_identifier_shape(key).is_err() || !valid {
                return Err("invalid environment rule parameter".into());
            }
        }
    }
    Ok(())
}
pub(super) fn access<'a, C>(
    section: &EnvironmentAccessRules<C>,
    selected: impl Iterator<Item = &'a str>,
) -> Result<(), String> {
    parameters(section, selected)?;
    for (id, rule) in &section.custom {
        rule.validate(id)?;
    }
    Ok(())
}
impl EnvironmentAccessRule {
    pub(crate) fn validate_structure(&self, id: &str) -> Result<(), String> {
        definition(id, &self.name, &self.condition)?;
        if !matches!(
            self.action,
            SecurityRuleAction::Allow | SecurityRuleAction::Ask | SecurityRuleAction::Block
        ) || self
            .priority
            .is_some_and(|priority| !(USER_PRIORITY_MIN..=USER_PRIORITY_MAX).contains(&priority))
        {
            return Err("environment access rules require a user priority and allow, ask or block".into());
        }
        Ok(())
    }
    pub fn validate(&self, id: &str) -> Result<(), String> {
        self.validate_structure(id)?;
        crate::security_rule_profile::validate_security_event_match(&self.condition)
            .map_err(|_| "invalid environment access rule condition".into())
    }
}
pub(super) fn detection<'a, C>(
    section: &EnvironmentDetectionRules<C>,
    selected: impl Iterator<Item = &'a str>,
) -> Result<(), String> {
    parameters(section, selected)?;
    for (id, rule) in &section.custom {
        rule.validate(id)?;
    }
    Ok(())
}
impl EnvironmentDetectionRule {
    pub(crate) fn validate_structure(&self, id: &str) -> Result<(), String> {
        definition(id, &self.name, &self.condition)
    }
    pub fn validate(&self, id: &str) -> Result<(), String> {
        self.validate_structure(id)?;
        crate::security_rule_profile::validate_security_event_match(&self.condition)
            .map_err(|_| "invalid environment detection rule condition".into())
    }
}

impl EnvironmentAccessRules {
    /// Validate a selected canonical section without requiring an unrelated
    /// workload specification. This does not verify catalog provenance.
    pub fn validate(&self) -> Result<(), String> {
        self.catalog.validate()?;
        access(self, self.catalog.selected.iter().map(|entry| entry.id.as_str()))
    }
}
impl EnvironmentDetectionRules {
    /// The same canonical limits and pin validation used by a full revision.
    pub fn validate(&self) -> Result<(), String> {
        self.catalog.validate()?;
        detection(self, self.catalog.selected.iter().map(|entry| entry.id.as_str()))
    }
}
