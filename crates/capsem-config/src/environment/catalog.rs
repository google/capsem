//! Editable catalog selectors are distinct from host-resolved immutable pins.
use super::*;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentCatalogSelection {
    pub id: String,
    pub version: String,
}

/// The registry owner verifies the OCI source and digest before constructing
/// a canonical revision. Format validation alone does not attest provenance.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentCatalogPin {
    pub id: String,
    pub version: String,
    pub catalog_reference: String,
    pub resolved_digest: String,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, bound(deserialize = "C: Deserialize<'de>"))]
pub struct EnvironmentCatalogSelections<C> {
    #[serde(default)]
    pub selected: Vec<C>,
}
impl<C> Default for EnvironmentCatalogSelections<C> {
    fn default() -> Self {
        Self { selected: Vec::new() }
    }
}

fn selection(id: &str, version: &str) -> Result<(), String> {
    if crate::validate_identifier_shape(id).is_err()
        || version.is_empty()
        || version.len() > capsem_proto::ENVIRONMENT_MAX_CATALOG_REFERENCE_BYTES
        || version.chars().any(|ch| ch.is_control() || ch.is_whitespace())
    {
        return Err("invalid environment catalog selection".into());
    }
    Ok(())
}
fn identities<'a>(ids: impl Iterator<Item = &'a str>) -> Result<(), String> {
    let mut seen = std::collections::BTreeSet::new();
    for id in ids {
        if seen.len() >= capsem_proto::ENVIRONMENT_MAX_CATALOG_SELECTIONS || !seen.insert(id) {
            return Err("duplicate or too many environment catalog selections".into());
        }
    }
    Ok(())
}
impl EnvironmentCatalogSelections<EnvironmentCatalogSelection> {
    pub(super) fn validate(&self) -> Result<(), String> {
        identities(self.selected.iter().map(|entry| entry.id.as_str()))?;
        for entry in &self.selected {
            selection(&entry.id, &entry.version)?;
        }
        Ok(())
    }
}
impl EnvironmentCatalogSelections<EnvironmentCatalogPin> {
    pub(super) fn validate(&self) -> Result<(), String> {
        identities(self.selected.iter().map(|entry| entry.id.as_str()))?;
        for entry in &self.selected {
            selection(&entry.id, &entry.version)?;
            let reference = &entry.catalog_reference;
            if reference.is_empty()
                || reference.len() > capsem_proto::ENVIRONMENT_MAX_CATALOG_REFERENCE_BYTES
                || reference.contains("://")
                || reference.chars().any(|ch| ch.is_control() || ch.is_whitespace())
                || !crate::validation::is_sha256_digest_shape(&entry.resolved_digest)
            {
                return Err("invalid environment catalog pin".into());
            }
        }
        Ok(())
    }
}
