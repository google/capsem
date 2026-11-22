//! Reject duplicate or excessive external map keys before reading replacements.
use super::{CatalogRuleParameter, McpCatalogRequirement, SkillFile};
use serde::de::{Error, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use std::{collections::BTreeMap, fmt, marker::PhantomData};

pub(super) fn rule_parameters<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<BTreeMap<String, CatalogRuleParameter>, D::Error> {
    unique_map(deserializer, capsem_proto::CATALOG_MAX_RULE_PARAMETERS)
}
pub(super) fn skill_files<'de, D: Deserializer<'de>>(deserializer: D) -> Result<BTreeMap<String, SkillFile>, D::Error> {
    unique_map(deserializer, capsem_proto::CATALOG_MAX_SKILL_FILES)
}
pub(super) fn mcp_requirements<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<BTreeMap<String, McpCatalogRequirement>, D::Error> {
    unique_map(deserializer, capsem_proto::ENVIRONMENT_MAX_CONFIGURATION_ENTRIES)
}
fn unique_map<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
    max: usize,
) -> Result<BTreeMap<String, T>, D::Error> {
    struct UniqueMap<T> {
        max: usize,
        marker: PhantomData<T>,
    }
    impl<'de, T: Deserialize<'de>> Visitor<'de> for UniqueMap<T> {
        type Value = BTreeMap<String, T>;
        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a bounded catalog object with unique keys")
        }
        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
            let mut values = BTreeMap::new();
            while let Some(key) = map.next_key::<String>()? {
                if values.len() >= self.max {
                    return Err(A::Error::custom("too many catalog map keys"));
                }
                if values.contains_key(&key) {
                    return Err(A::Error::custom("duplicate catalog map key"));
                }
                values.insert(key, map.next_value()?);
            }
            Ok(values)
        }
    }
    deserializer.deserialize_map(UniqueMap {
        max,
        marker: PhantomData,
    })
}
