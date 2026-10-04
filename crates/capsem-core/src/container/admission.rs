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
//!
//! The catalog is fetched, so a policy is built from the files alone and the
//! catalog joins it only when a decision needs it ([`ImagePolicy::with_catalog`]):
//! [`ImagePolicy::grants_source`] and [`ImagePolicy::grants`] answer from the
//! explicit grants, and a caller that gets `true` never fetches anything.

use std::path::PathBuf;

use anyhow::{bail, ensure, Context, Result};
use capsem_assets::oci::{admits, allows_source, ImageReference, ImageSelector, ResolvedImage, DEFAULT_CATALOG};
use capsem_config::{CatalogSetting, ImagePolicyConfig, SettingsFile};

/// Where the effective policy reads its catalog from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogSource {
    /// A registry-qualified OCI reference, usually a moving channel tag.
    pub reference: String,
    /// A PEM file trusted for the catalog's registry only.
    pub ca: Option<PathBuf>,
}

/// The effective image policy for one decision.
#[derive(Debug, Clone, Default)]
pub struct ImagePolicy {
    sources: Vec<ImageSelector>,
    admit: Vec<ImageSelector>,
    catalog_source: Option<CatalogSource>,
    catalog: Vec<ResolvedImage>,
}

impl ImagePolicy {
    /// The policy from the user's settings and the corp overlay, before any
    /// catalog has joined it. A selector or catalog setting that does not
    /// parse fails the whole policy: a typo must not silently narrow or widen
    /// it.
    pub fn from_files(settings: &SettingsFile, corp: &SettingsFile) -> Result<Self> {
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
            catalog_source: catalog_source(&config).with_context(|| format!("{owner} [images]"))?,
            catalog: Vec::new(),
        })
    }

    /// Where this policy reads its catalog from; `None` when the catalog is
    /// turned off, so only the explicit grants decide.
    pub fn catalog_source(&self) -> Option<&CatalogSource> {
        self.catalog_source.as_ref()
    }

    /// This policy with the catalog's supported images, read from
    /// [`Self::catalog_source`], joined to it.
    pub fn with_catalog(mut self, supported: impl IntoIterator<Item = ResolvedImage>) -> Self {
        self.catalog = supported.into_iter().collect();
        self
    }

    /// Whether `[images] sources` alone allows fetching `reference`. When it
    /// does, no catalog is needed to decide.
    pub fn grants_source(&self, reference: &ImageReference) -> bool {
        allows_source(&self.sources, reference)
    }

    /// Whether `[images] admit` alone admits `image`. When it does, no
    /// catalog is needed to decide.
    pub fn grants(&self, image: &ResolvedImage) -> bool {
        admits(&self.admit, image)
    }

    /// May bytes for `reference` be fetched? Checked before any registry
    /// access, so a refused source is never contacted.
    pub fn check_source(&self, reference: &ImageReference) -> Result<()> {
        let catalog_repository = self
            .catalog
            .iter()
            .any(|image| image.repository() == reference.repository());
        if catalog_repository || self.grants_source(reference) {
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
        if self.catalog.iter().any(|listed| listed.digest() == image.digest()) || self.grants(image) {
            return Ok(());
        }
        bail!("image {image} is not admitted: add it to [images] admit in settings.toml or corp.toml")
    }
}

fn catalog_source(config: &ImagePolicyConfig) -> Result<Option<CatalogSource>> {
    let reference = match &config.catalog {
        None | Some(CatalogSetting::Enabled(true)) => DEFAULT_CATALOG.to_owned(),
        Some(CatalogSetting::Enabled(false)) => {
            ensure!(
                config.catalog_ca.is_none(),
                "catalog_ca names a CA for a catalog that is turned off"
            );
            return Ok(None);
        }
        Some(CatalogSetting::Reference(reference)) => {
            capsem_assets::oci::image_reference(reference)
                .with_context(|| format!("catalog: {reference:?} is not a registry-qualified OCI reference"))?;
            reference.clone()
        }
    };
    let ca = config.catalog_ca.as_ref().map(PathBuf::from);
    if let Some(ca) = &ca {
        ensure!(
            ca.is_absolute(),
            "catalog_ca must be an absolute path: {}",
            ca.display()
        );
    }
    Ok(Some(CatalogSource { reference, ca }))
}

#[cfg(test)]
mod tests;
