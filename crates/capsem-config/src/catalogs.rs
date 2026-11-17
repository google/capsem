//! Shared environment catalog source contracts; fetching belongs to assets.
use serde::{Deserialize, Serialize};

pub const ENVIRONMENT_CATALOG_SCHEMA_VERSION: u16 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogKind {
    Skills,
    Rules,
    Mcp,
}
impl CatalogKind {
    pub const ALL: [Self; 3] = [Self::Skills, Self::Rules, Self::Mcp];
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Skills => "skills",
            Self::Rules => "rules",
            Self::Mcp => "mcp",
        }
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogSourceDescriptor {
    pub reference: String,
    pub schema_version: u16,
}
impl CatalogSourceDescriptor {
    /// Admission bounds and schema support only. Assets additionally normalizes
    /// the OCI reference and verifies every fetched manifest/layer digest.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.schema_version != ENVIRONMENT_CATALOG_SCHEMA_VERSION {
            return Err("unsupported environment catalog schema version");
        }
        if self.reference.is_empty()
            || self.reference.len() > capsem_proto::ENVIRONMENT_MAX_CATALOG_REFERENCE_BYTES
            || self.reference.contains("://")
            || self.reference.chars().any(|ch| ch.is_control() || ch.is_whitespace())
        {
            return Err("invalid environment catalog source reference");
        }
        Ok(())
    }
}
impl std::fmt::Debug for CatalogSourceDescriptor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CatalogSourceDescriptor")
            .field("schema_version", &self.schema_version)
            .field("reference_bytes", &self.reference.len())
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CatalogSources {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skills: Option<CatalogSourceDescriptor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rules: Option<CatalogSourceDescriptor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp: Option<CatalogSourceDescriptor>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogSourceOrigin {
    Manifest,
    Corporate,
}
#[derive(Clone, Copy, Debug)]
pub struct SelectedCatalogSource<'a> {
    pub source: &'a CatalogSourceDescriptor,
    pub origin: CatalogSourceOrigin,
}
impl CatalogSources {
    pub fn source(&self, kind: CatalogKind) -> Option<&CatalogSourceDescriptor> {
        match kind {
            CatalogKind::Skills => self.skills.as_ref(),
            CatalogKind::Rules => self.rules.as_ref(),
            CatalogKind::Mcp => self.mcp.as_ref(),
        }
    }
    /// Select before attempting validation or I/O. A failing corporate choice
    /// remains the chosen source; callers report it unavailable, never retry
    /// the manifest source as an implicit substitute.
    pub fn effective_source<'a>(&'a self, corporate: &'a Self, kind: CatalogKind) -> Option<SelectedCatalogSource<'a>> {
        if let Some(source) = corporate.source(kind) {
            return Some(SelectedCatalogSource {
                source,
                origin: CatalogSourceOrigin::Corporate,
            });
        }
        self.source(kind).map(|source| SelectedCatalogSource {
            source,
            origin: CatalogSourceOrigin::Manifest,
        })
    }
}

#[cfg(test)]
mod tests;
