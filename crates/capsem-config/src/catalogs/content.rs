//! Untrusted OCI catalog data contracts. Integrity, provenance verification,
//! source selection and launch/permission authority belong to their owners.
use super::{CatalogKind, ENVIRONMENT_CATALOG_SCHEMA_VERSION};
use capsem_proto::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const CATALOG_RUNTIME_CONTRACT: u32 = 1;

/// A parser must enforce CATALOG_MAX_DOCUMENT_BYTES before decoding. This
/// validation handles typed content, never infers trust from publisher fields,
/// and does not substitute for that external encoded document byte ceiling.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogDocument<E> {
    pub schema_version: u16,
    pub kind: CatalogKind,
    pub entries: Vec<E>,
}

pub trait CatalogEntry {
    const KIND: CatalogKind;
    fn metadata(&self) -> &CatalogEntryMetadata;
    fn validate_content(&self) -> Result<(), &'static str>;
}

impl<E: CatalogEntry> CatalogDocument<E> {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.schema_version != ENVIRONMENT_CATALOG_SCHEMA_VERSION || self.kind != E::KIND {
            return Err("unsupported catalog kind or schema version");
        }
        if self.entries.len() > CATALOG_MAX_ENTRIES {
            return Err("too many catalog entries");
        }
        let mut identities = BTreeSet::new();
        for entry in &self.entries {
            let metadata = entry.metadata();
            metadata.validate()?;
            if !identities.insert((&metadata.id, &metadata.version)) {
                return Err("duplicate catalog entry identity");
            }
            entry.validate_content()?;
        }
        Ok(())
    }
}

#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogCompatibility {
    pub min_runtime_contract: u32,
}
impl CatalogCompatibility {
    pub fn is_supported(self) -> bool {
        (1..=CATALOG_RUNTIME_CONTRACT).contains(&self.min_runtime_contract)
    }
}

#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogEntryStatus {
    Active,
    Deprecated,
    Revoked,
}

#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogEntryMetadata {
    pub id: String,
    pub version: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Publisher-claimed labels, not a verified signing identity.
    pub publisher: String,
    pub provenance: String,
    pub license: String,
    pub compatibility: CatalogCompatibility,
    pub status: CatalogEntryStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_reason: Option<String>,
}

fn text(value: &str, max: usize, multiline: bool) -> bool {
    value.len() <= max
        && !value
            .chars()
            .any(|ch| ch.is_control() && !(multiline && matches!(ch, '\n' | '\r' | '\t')))
}
fn label(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && text(value, max, false)
}
impl CatalogEntryMetadata {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.id.len() > CATALOG_MAX_IDENTIFIER_BYTES
            || crate::validate_identifier_shape(&self.id).is_err()
            || self.version.is_empty()
            || self.version.len() > CATALOG_MAX_VERSION_BYTES
            || self.version.chars().any(|ch| ch.is_control() || ch.is_whitespace())
            || !label(&self.name, CATALOG_MAX_NAME_BYTES)
            || !text(&self.description, CATALOG_MAX_DESCRIPTION_BYTES, true)
            || !label(&self.publisher, CATALOG_MAX_NAME_BYTES)
            || !label(&self.provenance, CATALOG_MAX_PROVENANCE_BYTES)
            || !label(&self.license, CATALOG_MAX_NAME_BYTES)
            || self.compatibility.min_runtime_contract == 0
        {
            return Err("invalid catalog entry metadata");
        }
        if self.tags.len() > CATALOG_MAX_TAGS {
            return Err("too many catalog entry tags");
        }
        let mut tags = BTreeSet::new();
        for tag in &self.tags {
            if tag.len() > CATALOG_MAX_TAG_BYTES || crate::validate_identifier_shape(tag).is_err() || !tags.insert(tag)
            {
                return Err("invalid or duplicate catalog entry tag");
            }
        }
        match (self.status, self.status_reason.as_deref()) {
            (CatalogEntryStatus::Active, None) => Ok(()),
            (CatalogEntryStatus::Deprecated | CatalogEntryStatus::Revoked, Some(reason))
                if label(reason, CATALOG_MAX_DESCRIPTION_BYTES) =>
            {
                Ok(())
            }
            _ => Err("catalog lifecycle state requires a bounded reason"),
        }
    }

    /// A valid descriptor may remain enumerable for diagnosis even when its
    /// runtime contract is newer than this client. This predicate alone grants
    /// no authority and does not verify integrity or current corporate policy.
    pub fn is_selectable(&self) -> bool {
        self.status != CatalogEntryStatus::Revoked && self.compatibility.is_supported()
    }
}
