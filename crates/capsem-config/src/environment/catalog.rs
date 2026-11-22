//! Editable catalog selectors are distinct from host-resolved immutable pins.
use super::*;

#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentCatalogSelection {
    pub id: String,
    pub version: String,
    /// Optional precondition from a catalog detail read. The host still verifies the selection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_catalog: Option<EnvironmentCatalogExpectation>,
}

#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentCatalogExpectation {
    pub reference: String,
    pub digest: String,
}

impl EnvironmentCatalogExpectation {
    pub(super) fn matches(&self, pin: &EnvironmentCatalogPin) -> bool {
        self.reference == pin.catalog_reference && self.digest == pin.resolved_digest
    }
}

impl EnvironmentCatalogSelection {
    pub fn expects(&self, pin: &EnvironmentCatalogPin) -> bool {
        self.expected_catalog
            .as_ref()
            .is_none_or(|expected| expected.matches(pin))
    }
}

/// The registry owner verifies the OCI source and digest before constructing
/// a canonical revision. Format validation alone does not attest provenance.
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentCatalogPin {
    pub id: String,
    pub version: String,
    pub catalog_reference: String,
    pub resolved_digest: String,
}

#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
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
            if let Some(expected) = &entry.expected_catalog {
                catalog_identity(&expected.reference, &expected.digest)?;
            }
        }
        Ok(())
    }
}
impl EnvironmentCatalogSelections<EnvironmentCatalogPin> {
    pub(super) fn validate(&self) -> Result<(), String> {
        identities(self.selected.iter().map(|entry| entry.id.as_str()))?;
        for entry in &self.selected {
            selection(&entry.id, &entry.version)?;
            catalog_identity(&entry.catalog_reference, &entry.resolved_digest)?;
        }
        Ok(())
    }
}

fn catalog_identity(reference: &str, digest: &str) -> Result<(), String> {
    if reference.is_empty()
        || reference.len() > capsem_proto::ENVIRONMENT_MAX_CATALOG_REFERENCE_BYTES
        || reference.contains("://")
        || reference.chars().any(|ch| ch.is_control() || ch.is_whitespace())
        || !crate::validation::is_sha256_digest_shape(digest)
    {
        return Err("invalid environment catalog pin or expectation".into());
    }
    Ok(())
}
