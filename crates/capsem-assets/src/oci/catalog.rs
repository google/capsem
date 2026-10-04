//! The image catalog: which official images a channel offers, and therefore
//! which digests default admission admits.
//!
//! A catalog is a small JSON document pushed as an OCI artifact to
//! `ghcr.io/google/capsem/catalog:<channel>`. Its manifest carries one layer
//! of media type [`CATALOG_MEDIA_TYPE`] holding the document. The channel tag
//! moves; each catalog version is immutable by manifest digest, which is how
//! [`super::Puller::fetch_catalog`] identifies it.
//!
//! Every image is pinned by digest, and each entry lists its versions oldest
//! to newest. [`Catalog::resolve`] picks the newest version this runtime can
//! run; [`Catalog::supported`] is every listed digest. A digest is revoked by
//! publishing a catalog that no longer lists it: there is no deny list.
//!
//! The writer is `images/ci/catalog.py`; this parser accepts exactly the shape
//! it publishes, including the per-architecture `obom` and `erofs` maps.

use std::{collections::BTreeMap, fmt};

use anyhow::{bail, ensure, Context, Result};
use serde::Deserialize;

use super::selector::{Digest, ImageReference, ResolvedImage};
use crate::asset_manager::PackageArchitecture;

/// The catalog contract this runtime implements. A catalog version declaring
/// a higher contract needs a newer runtime and is never resolved here.
pub const RUNTIME_CONTRACT: u32 = 1;

/// The catalog a runtime reads unless `[images] catalog` names another, such
/// as an enterprise mirror.
pub const DEFAULT_CATALOG: &str = "ghcr.io/google/capsem/catalog:stable";

/// Media type of the single layer that holds the catalog document.
pub const CATALOG_MEDIA_TYPE: &str = "application/vnd.capsem.catalog.v1+json";

const SCHEMA_VERSION: u32 = 1;
const PLATFORM_OS: &str = "linux/";
const ARCHITECTURES: [PackageArchitecture; 2] = [PackageArchitecture::Arm64, PackageArchitecture::Amd64];

/// The release channel a catalog describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    Stable,
    Nightly,
}

impl fmt::Display for Channel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Stable => "stable",
            Self::Nightly => "nightly",
        })
    }
}

/// A validated catalog document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Catalog {
    channel: Channel,
    generated_at: Option<String>,
    entries: BTreeMap<String, CatalogEntry>,
}

/// One named image and its versions, oldest to newest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogEntry {
    description: String,
    icon: Option<String>,
    versions: Vec<CatalogVersion>,
}

/// One immutable image version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogVersion {
    image: ResolvedImage,
    platforms: Vec<PackageArchitecture>,
    contract: u32,
    obom: BTreeMap<PackageArchitecture, Digest>,
    erofs: BTreeMap<PackageArchitecture, Digest>,
}

impl Catalog {
    /// Parse and validate a catalog document. Unknown fields are refused.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let document: Document = serde_json::from_slice(bytes).context("invalid catalog document")?;
        ensure!(
            document.schema_version == SCHEMA_VERSION,
            "unsupported catalog schema_version {}, expected {SCHEMA_VERSION}",
            document.schema_version
        );
        let entries = document
            .entries
            .into_iter()
            .map(|(name, entry)| {
                let entry = CatalogEntry::validate(&name, entry).with_context(|| format!("catalog entry {name:?}"))?;
                Ok((name, entry))
            })
            .collect::<Result<_>>()?;
        Ok(Self {
            channel: document.channel,
            generated_at: document.generated_at,
            entries,
        })
    }

    pub fn channel(&self) -> Channel {
        self.channel
    }

    /// When the publishing workflow wrote this version, as it recorded it.
    pub fn generated_at(&self) -> Option<&str> {
        self.generated_at.as_deref()
    }

    pub fn entries(&self) -> &BTreeMap<String, CatalogEntry> {
        &self.entries
    }

    pub fn entry(&self, name: &str) -> Option<&CatalogEntry> {
        self.entries.get(name)
    }

    /// The newest version of `name` built for `architecture` whose contract
    /// this runtime implements.
    pub fn resolve(
        &self,
        name: &str,
        architecture: PackageArchitecture,
        runtime_contract: u32,
    ) -> Result<ResolvedImage> {
        let entry = self
            .entry(name)
            .with_context(|| format!("image {name:?} is not in the {} catalog", self.channel))?;
        entry
            .versions
            .iter()
            .rev()
            .find(|version| version.platforms.contains(&architecture) && version.contract <= runtime_contract)
            .map(|version| version.image.clone())
            .with_context(|| {
                format!(
                    "no version of {name:?} in the {} catalog runs on linux/{} at runtime contract {runtime_contract}",
                    self.channel,
                    architecture.as_str()
                )
            })
    }

    /// Every listed image, in every version. Default admission admits exactly
    /// these; a digest absent from the current catalog is revoked.
    pub fn supported(&self) -> impl Iterator<Item = &ResolvedImage> + '_ {
        self.entries
            .values()
            .flat_map(|entry| entry.versions.iter().map(|version| &version.image))
    }
}

impl CatalogEntry {
    fn validate(name: &str, entry: EntryDocument) -> Result<Self> {
        ensure!(
            is_catalog_name(name),
            "invalid catalog entry name: use 1-63 of [a-z0-9-], starting with a letter or digit"
        );
        ensure!(!entry.versions.is_empty(), "entry lists no versions");
        let mut versions: Vec<CatalogVersion> = Vec::with_capacity(entry.versions.len());
        for (index, version) in entry.versions.into_iter().enumerate() {
            let version = CatalogVersion::validate(version).with_context(|| format!("version {index}"))?;
            ensure!(
                !versions
                    .iter()
                    .any(|seen| seen.image.digest() == version.image.digest()),
                "duplicate digest {} in version {index}",
                version.image.digest()
            );
            versions.push(version);
        }
        Ok(Self {
            description: entry.description,
            icon: entry.icon,
            versions,
        })
    }

    pub fn description(&self) -> &str {
        &self.description
    }

    pub fn icon(&self) -> Option<&str> {
        self.icon.as_deref()
    }

    /// Oldest to newest.
    pub fn versions(&self) -> &[CatalogVersion] {
        &self.versions
    }
}

impl CatalogVersion {
    fn validate(version: VersionDocument) -> Result<Self> {
        let reference: ImageReference = version
            .image
            .parse()
            .with_context(|| format!("invalid image {:?}", version.image))?;
        let digest = reference
            .digest()
            .cloned()
            .with_context(|| format!("image {:?} must be pinned by @sha256: digest", version.image))?;
        let image = reference.resolve(digest)?;
        ensure!(!version.platforms.is_empty(), "version lists no platform");
        let mut platforms = Vec::with_capacity(version.platforms.len());
        for platform in &version.platforms {
            let architecture = parse_platform(platform)?;
            ensure!(!platforms.contains(&architecture), "platform {platform:?} listed twice");
            platforms.push(architecture);
        }
        let obom = per_architecture("obom", version.obom, &platforms)?;
        let erofs = per_architecture("erofs", version.erofs, &platforms)?;
        Ok(Self {
            image,
            platforms,
            contract: version.contract,
            obom,
            erofs,
        })
    }

    pub fn image(&self) -> &ResolvedImage {
        &self.image
    }

    pub fn platforms(&self) -> &[PackageArchitecture] {
        &self.platforms
    }

    /// The runtime contract this version needs.
    pub fn contract(&self) -> u32 {
        self.contract
    }

    /// The OBOM digest for `architecture`, when published.
    pub fn obom(&self, architecture: PackageArchitecture) -> Option<&Digest> {
        self.obom.get(&architecture)
    }

    /// The prebuilt EROFS image digest for `architecture`, when published.
    pub fn erofs(&self, architecture: PackageArchitecture) -> Option<&Digest> {
        self.erofs.get(&architecture)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    schema_version: u32,
    channel: Channel,
    /// When the publishing workflow wrote this version (RFC 3339 UTC).
    /// Informational: the manifest digest, not this, identifies a catalog.
    #[serde(default)]
    generated_at: Option<String>,
    entries: BTreeMap<String, EntryDocument>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EntryDocument {
    description: String,
    #[serde(default)]
    icon: Option<String>,
    versions: Vec<VersionDocument>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VersionDocument {
    image: String,
    platforms: Vec<String>,
    contract: u32,
    #[serde(default)]
    obom: BTreeMap<PackageArchitecture, String>,
    #[serde(default)]
    erofs: BTreeMap<PackageArchitecture, String>,
}

/// Validate a per-architecture digest map such as `obom` or `erofs`: every
/// key is an architecture the version is built for.
fn per_architecture(
    field: &str,
    digests: BTreeMap<PackageArchitecture, String>,
    platforms: &[PackageArchitecture],
) -> Result<BTreeMap<PackageArchitecture, Digest>> {
    digests
        .into_iter()
        .map(|(architecture, digest)| {
            let architecture_name = architecture.as_str();
            ensure!(
                platforms.contains(&architecture),
                "{field} names {architecture_name} but the version is not built for it"
            );
            let digest =
                Digest::parse(&digest).with_context(|| format!("invalid {field} digest for {architecture_name}"))?;
            Ok((architecture, digest))
        })
        .collect()
}

fn parse_platform(platform: &str) -> Result<PackageArchitecture> {
    let Some(architecture) = platform.strip_prefix(PLATFORM_OS) else {
        bail!("unsupported platform {platform:?}: expected linux/<architecture>");
    };
    ARCHITECTURES
        .into_iter()
        .find(|known| known.as_str() == architecture)
        .with_context(|| format!("unsupported platform {platform:?}: architecture must be arm64 or amd64"))
}

/// Whether `name` has the shape of a catalog entry name,
/// `[a-z0-9][a-z0-9-]{0,62}`. Such a name never contains `/`, `:` or `@`, so
/// it is never a registry-qualified reference; it can look like a Docker Hub
/// short name (`redis`), which is why a catalog name is looked up before
/// anything is parsed as a reference.
pub fn is_catalog_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    (1..=63).contains(&bytes.len())
        && bytes[0] != b'-'
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}

#[cfg(test)]
mod tests;
