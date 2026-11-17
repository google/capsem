//! Presets resolve typed literal parameters into the existing rule language.
use super::{CatalogEntry, CatalogEntryMetadata, CatalogEntryStatus, CatalogKind};
use crate::environment::{EnvironmentAccessRule, EnvironmentDetectionRule, EnvironmentRuleParameter};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

mod parameters;
pub use parameters::CatalogRuleParameter;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleCatalogEntry {
    pub metadata: CatalogEntryMetadata,
    #[serde(default)]
    pub parameters: BTreeMap<String, CatalogRuleParameter>,
    /// A withdrawn identity may be published as a content-free tombstone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition: Option<RuleCatalogDefinition>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RuleCatalogDefinition {
    Access { rule: EnvironmentAccessRule },
    Detection { rule: EnvironmentDetectionRule },
}

#[derive(Clone, PartialEq, Eq)]
pub enum ResolvedCatalogRule {
    Access(EnvironmentAccessRule),
    Detection(EnvironmentDetectionRule),
}

impl CatalogEntry for RuleCatalogEntry {
    const KIND: CatalogKind = CatalogKind::Rules;
    fn metadata(&self) -> &CatalogEntryMetadata {
        &self.metadata
    }
    fn validate_content(&self) -> Result<(), &'static str> {
        if self.definition.is_none() {
            return if self.metadata.status == CatalogEntryStatus::Revoked && self.parameters.is_empty() {
                Ok(())
            } else {
                Err("catalog rule definition is required")
            };
        }
        self.resolve_definition(&BTreeMap::new()).map(|_| ())
    }
}

impl RuleCatalogEntry {
    /// Format and compatibility checks only. The registry owner must additionally
    /// verify the pinned artifact, current revocation and corporate policy.
    pub fn resolve(
        &self,
        values: &BTreeMap<String, EnvironmentRuleParameter>,
    ) -> Result<ResolvedCatalogRule, &'static str> {
        self.metadata.validate()?;
        if !self.metadata.is_selectable() {
            return Err("catalog rule is revoked or incompatible");
        }
        self.resolve_definition(values)
    }

    fn resolve_definition(
        &self,
        values: &BTreeMap<String, EnvironmentRuleParameter>,
    ) -> Result<ResolvedCatalogRule, &'static str> {
        if self.parameters.len() > capsem_proto::CATALOG_MAX_RULE_PARAMETERS
            || values.len() > self.parameters.len()
            || values.keys().any(|key| !self.parameters.contains_key(key))
        {
            return Err("unknown or too many catalog rule parameters");
        }
        for (key, parameter) in &self.parameters {
            if crate::validate_identifier_shape(key).is_err() {
                return Err("invalid catalog rule parameter identity");
            }
            parameter.validate()?;
        }
        match self
            .definition
            .as_ref()
            .ok_or("catalog rule definition is unavailable")?
        {
            RuleCatalogDefinition::Access { rule } => {
                rule.validate_structure(&self.metadata.id)
                    .map_err(|_| "invalid catalog access rule")?;
                let mut rule = rule.clone();
                rule.condition = parameters::render(&rule.condition, &self.parameters, values)?;
                rule.validate(&self.metadata.id)
                    .map_err(|_| "invalid catalog access rule condition")?;
                Ok(ResolvedCatalogRule::Access(rule))
            }
            RuleCatalogDefinition::Detection { rule } => {
                rule.validate_structure(&self.metadata.id)
                    .map_err(|_| "invalid catalog detection rule")?;
                let mut rule = rule.clone();
                rule.condition = parameters::render(&rule.condition, &self.parameters, values)?;
                rule.validate(&self.metadata.id)
                    .map_err(|_| "invalid catalog detection rule condition")?;
                Ok(ResolvedCatalogRule::Detection(rule))
            }
        }
    }
}
