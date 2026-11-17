//! Complete UTF-8 skill manifests. Declared digest syntax is validated here;
//! Core's integrity owner verifies actual file hashes before serving content.
use super::{CatalogEntry, CatalogEntryMetadata, CatalogEntryStatus, CatalogKind};
use capsem_proto::{
    CATALOG_MAX_SKILL_FILES, CATALOG_MAX_SKILL_FILE_BYTES, CATALOG_MAX_SKILL_PATH_BYTES, CATALOG_MAX_SKILL_TOTAL_BYTES,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillCatalogEntry {
    pub metadata: CatalogEntryMetadata,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<SkillPayload>,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillPayload {
    #[serde(deserialize_with = "super::collections::skill_files")]
    pub files: BTreeMap<String, SkillFile>,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillFile {
    pub sha256: String,
    pub content: String,
}
impl CatalogEntry for SkillCatalogEntry {
    const KIND: CatalogKind = CatalogKind::Skills;
    fn metadata(&self) -> &CatalogEntryMetadata {
        &self.metadata
    }
    fn validate_content(&self) -> Result<(), &'static str> {
        match &self.payload {
            Some(payload) => payload.validate(&self.metadata.id),
            None if self.metadata.status == CatalogEntryStatus::Revoked => Ok(()),
            None => Err("skill catalog payload is required"),
        }
    }
}
fn text(content: &str) -> bool {
    !content
        .chars()
        .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t'))
}
fn path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= CATALOG_MAX_SKILL_PATH_BYTES
        && text(path)
        && !path.chars().any(|ch| ch.is_control() || matches!(ch, '\\' | ':'))
        && path.split('/').all(|component| !matches!(component, "" | "." | ".."))
}
impl SkillPayload {
    pub fn validate(&self, id: &str) -> Result<(), &'static str> {
        if self.files.len() > CATALOG_MAX_SKILL_FILES {
            return Err("too many skill files");
        }
        let main = self.files.get("SKILL.md").ok_or("skill requires SKILL.md")?;
        let mut total = 0usize;
        for (name, file) in &self.files {
            if !path(name)
                || !crate::validation::is_sha256_digest_shape(&file.sha256)
                || file.content.len() > CATALOG_MAX_SKILL_FILE_BYTES
                || !text(&file.content)
            {
                return Err("invalid skill file path, digest declaration or text");
            }
            total = total
                .checked_add(file.content.len())
                .ok_or("skill content exceeds its aggregate budget")?;
            if total > CATALOG_MAX_SKILL_TOTAL_BYTES {
                return Err("skill content exceeds its aggregate budget");
            }
        }
        validate_frontmatter(&main.content, id)
    }
}

/// This is an external SKILL.md YAML frontmatter contract. Advisory tool text
/// remains content: this parser neither resolves nor grants those capabilities.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Frontmatter {
    name: String,
    description: String,
    license: Option<String>,
    compatibility: Option<String>,
    #[serde(default)]
    metadata: BTreeMap<String, String>,
    #[serde(rename = "allowed-tools")]
    allowed_tools: Option<String>,
}
fn validate_frontmatter(content: &str, id: &str) -> Result<(), &'static str> {
    let mut lines = content.lines();
    if lines.next() != Some("---") {
        return Err("skill requires YAML frontmatter");
    }
    let mut header = String::new();
    let mut closed = false;
    for line in &mut lines {
        if line == "---" {
            closed = true;
            break;
        }
        header.push_str(line);
        header.push('\n');
    }
    if !closed || lines.all(|line| line.trim().is_empty()) {
        return Err("skill requires complete frontmatter and instructions");
    }
    let metadata: Frontmatter = serde_saphyr::from_str(&header).map_err(|_| "invalid skill YAML frontmatter")?;
    if metadata.name != id
        || crate::validate_identifier_shape(&metadata.name).is_err()
        || metadata.description.trim().is_empty()
        || metadata.description.len() > capsem_proto::CATALOG_MAX_DESCRIPTION_BYTES
        || !text(&metadata.description)
        || metadata.metadata.len() > capsem_proto::ENVIRONMENT_MAX_CONFIGURATION_ENTRIES
    {
        return Err("invalid skill frontmatter metadata");
    }
    for (value, max) in [
        (metadata.license.as_deref(), capsem_proto::CATALOG_MAX_NAME_BYTES),
        (
            metadata.compatibility.as_deref(),
            capsem_proto::CATALOG_MAX_PROVENANCE_BYTES,
        ),
        (
            metadata.allowed_tools.as_deref(),
            capsem_proto::ENVIRONMENT_MAX_CONFIGURATION_TEXT_BYTES,
        ),
    ] {
        if value.is_some_and(|value| value.len() > max || !text(value)) {
            return Err("invalid skill advisory metadata");
        }
    }
    for (key, value) in &metadata.metadata {
        if crate::validate_identifier_shape(key).is_err()
            || value.len() > capsem_proto::ENVIRONMENT_MAX_CONFIGURATION_TEXT_BYTES
            || !text(value)
        {
            return Err("invalid skill auxiliary metadata");
        }
    }
    Ok(())
}
