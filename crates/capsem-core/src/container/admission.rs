//! Which images a session may fetch and run.
//!
//! Two checks at two times, from one policy (#289):
//!
//! - **Sources** -- where image bytes may come from -- are checked against the
//!   reference as requested, before any registry access.
//! - **Admission** -- which images may execute -- is checked against the
//!   resolved digest, after the pull and before anything is staged.
//!
//! The policy is the installation's `[images]` (corp's replaces the user's)
//! plus the catalog: every digest the catalog lists as supported is admitted,
//! wherever it was fetched from, and its repository is a source. Anything
//! else is denied, so with no `[images]` exactly the catalog runs.

use anyhow::{bail, Context, Result};
use capsem_assets::oci::{admits, allows_source, ImageReference, ImageSelector, ResolvedImage};
use capsem_config::{ImagePolicyConfig, SettingsFile};

/// The effective image policy for one decision.
#[derive(Debug, Clone, Default)]
pub struct ImagePolicy {
    sources: Vec<ImageSelector>,
    admit: Vec<ImageSelector>,
    catalog: Vec<ResolvedImage>,
}

impl ImagePolicy {
    /// The policy from the user's settings, the corp overlay and the
    /// catalog's supported images. A selector that does not parse fails the
    /// whole policy: a typo must not silently narrow or widen it.
    pub fn from_files(settings: &SettingsFile, corp: &SettingsFile, catalog: Vec<ResolvedImage>) -> Result<Self> {
        let (config, owner) = match (&corp.images, &settings.images) {
            (Some(config), _) => (config.clone(), "corp.toml"),
            (None, Some(config)) => (config.clone(), "settings.toml"),
            (None, None) => (ImagePolicyConfig::default(), "settings.toml"),
        };
        let parse = |field: &str, values: &[String]| -> Result<Vec<ImageSelector>> {
            values
                .iter()
                .map(|value| {
                    value
                        .parse::<ImageSelector>()
                        .with_context(|| format!("{owner} [images] {field}: {value:?}"))
                })
                .collect()
        };
        Ok(Self {
            sources: parse("sources", &config.sources)?,
            admit: parse("admit", &config.admit)?,
            catalog,
        })
    }

    /// May bytes for `reference` be fetched? Checked before any registry
    /// access, so a refused source is never contacted.
    pub fn check_source(&self, reference: &ImageReference) -> Result<()> {
        let catalog_repository = self
            .catalog
            .iter()
            .any(|image| image.repository() == reference.repository());
        if catalog_repository || allows_source(&self.sources, reference) {
            return Ok(());
        }
        bail!(
            "image source {} is not allowed: add it to [images] sources in settings.toml or corp.toml",
            reference.repository()
        )
    }

    /// May `image` run? Checked on the resolved digest, never on a tag. A
    /// catalog digest is admitted from any repository: a mirror serves the
    /// same bytes, and the digest is their identity.
    pub fn admit(&self, image: &ResolvedImage) -> Result<()> {
        if self.catalog.iter().any(|listed| listed.digest() == image.digest()) || admits(&self.admit, image) {
            return Ok(());
        }
        bail!("image {image} is not admitted: add it to [images] admit in settings.toml or corp.toml")
    }
}

#[cfg(test)]
mod tests;
