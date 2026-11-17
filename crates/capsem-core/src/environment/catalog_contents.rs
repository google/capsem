//! OCI catalog integrity and typed content admission, never caller approval.
use capsem_assets::{
    catalogs::ValidatedCatalogSource,
    oci::{Digest, Puller},
};
use capsem_config::{
    catalogs::{CatalogDocument, CatalogEntry, RuleCatalogEntry, SkillCatalogEntry},
    environment::EnvironmentCatalogPin,
};
use serde::de::DeserializeOwned;
use std::{fmt, path::Path};

mod sealed {
    pub trait Sealed {}
    impl Sealed for capsem_config::catalogs::RuleCatalogEntry {}
    impl Sealed for capsem_config::catalogs::SkillCatalogEntry {}
}

/// Only the content schemas whose integrity contracts this owner implements.
/// MCP launch catalogs require a separate publisher-signing decision.
pub trait CatalogContentEntry: CatalogEntry + DeserializeOwned + Send + sealed::Sealed + 'static {
    fn verify_contents(&self) -> Result<(), CatalogContentError>;
}
impl CatalogContentEntry for RuleCatalogEntry {
    fn verify_contents(&self) -> Result<(), CatalogContentError> {
        Ok(())
    }
}
impl CatalogContentEntry for SkillCatalogEntry {
    fn verify_contents(&self) -> Result<(), CatalogContentError> {
        if let Some(payload) = &self.payload {
            for file in payload.files.values() {
                // The published skill file manifest specifies SHA-256 over
                // exact UTF-8 contents. This detects inconsistent content; it
                // does not authenticate a publisher who supplied both values.
                let digest = format!("sha256:{:x}", capsem_crypto::Sha256::digest(file.content.as_bytes()));
                if digest != file.sha256 {
                    return Err(CatalogContentError::IntegrityMismatch);
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CatalogContentError {
    WrongKind,
    TooLarge,
    InvalidDocument,
    InvalidContent,
    IntegrityMismatch,
    Unavailable,
    WorkerFailed,
    UnknownEntry,
    UnusableEntry,
    PinMismatch,
}
impl fmt::Display for CatalogContentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::WrongKind => "catalog kind does not match the requested content",
            Self::TooLarge => "catalog document exceeds its byte limit",
            Self::InvalidDocument => "invalid catalog document",
            Self::InvalidContent => "invalid catalog content",
            Self::IntegrityMismatch => "catalog file integrity mismatch",
            Self::Unavailable => "catalog content is unavailable",
            Self::WorkerFailed => "catalog content worker failed",
            Self::UnknownEntry => "catalog entry is unavailable",
            Self::UnusableEntry => "catalog entry is revoked or incompatible",
            Self::PinMismatch => "catalog selection does not match its pinned source and content",
        })
    }
}
impl std::error::Error for CatalogContentError {}

/// Immutable typed content associated with the source actually fetched and
/// its OCI-verified manifest digest. This is no approval, signature assertion,
/// or assertion about a later refresh's revocations/corporate restrictions.
pub struct CatalogContents<E: CatalogContentEntry> {
    source: ValidatedCatalogSource,
    digest: Digest,
    entries: Vec<E>,
}
pub type RuleCatalogContents = CatalogContents<RuleCatalogEntry>;
pub type SkillCatalogContents = CatalogContents<SkillCatalogEntry>;
impl<E: CatalogContentEntry> fmt::Debug for CatalogContents<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CatalogContents")
            .field("kind", &E::KIND)
            .field("digest", &self.digest)
            .field("entry_count", &self.entries.len())
            .finish_non_exhaustive()
    }
}
impl<E: CatalogContentEntry> CatalogContents<E> {
    /// Explicit refresh work only; status/info polls read an already obtained
    /// value. Assets owns network deadlines, authentication and verified cache
    /// reuse. The parser runs off the asynchronous executor, without a second
    /// fetcher or cache. Source identity is captured by this same fetch call.
    pub async fn fetch(
        puller: &Puller,
        source: &ValidatedCatalogSource,
        staging_parent: &Path,
    ) -> Result<Self, CatalogContentError> {
        if source.kind() != E::KIND {
            return Err(CatalogContentError::WrongKind);
        }
        let source = source.clone();
        let artifact = puller
            .fetch_environment_catalog(&source, staging_parent)
            .await
            .map_err(|_| CatalogContentError::Unavailable)?;
        if artifact.kind() != E::KIND {
            return Err(CatalogContentError::WrongKind);
        }
        let digest = artifact.digest().clone();
        tokio::task::spawn_blocking(move || Self::decode(source, digest, artifact.into_bytes()))
            .await
            .map_err(|_| CatalogContentError::WorkerFailed)?
    }

    fn decode(source: ValidatedCatalogSource, digest: Digest, bytes: Vec<u8>) -> Result<Self, CatalogContentError> {
        if source.kind() != E::KIND {
            return Err(CatalogContentError::WrongKind);
        }
        if bytes.len() > capsem_proto::CATALOG_MAX_DOCUMENT_BYTES {
            return Err(CatalogContentError::TooLarge);
        }
        if bytes.iter().find(|byte| !byte.is_ascii_whitespace()) != Some(&b'{') {
            return Err(CatalogContentError::InvalidDocument);
        }
        // Independently published OCI catalog documents use JSON. Decode once
        // directly into their bounded typed schema, never an intermediary tree.
        let document: CatalogDocument<E> =
            serde_json::from_slice(&bytes).map_err(|_| CatalogContentError::InvalidDocument)?;
        document.validate().map_err(|_| CatalogContentError::InvalidContent)?;
        for entry in &document.entries {
            entry.verify_contents()?;
        }
        Ok(Self {
            source,
            digest,
            entries: document.entries,
        })
    }

    pub fn source(&self) -> &ValidatedCatalogSource {
        &self.source
    }
    pub fn digest(&self) -> &Digest {
        &self.digest
    }
    pub fn entries(&self) -> &[E] {
        &self.entries
    }

    /// Host resolution metadata for review. Selection never installs content
    /// or grants the entry's advertised network, tool or credential access.
    pub fn pin(&self, id: &str, version: &str) -> Result<EnvironmentCatalogPin, CatalogContentError> {
        let entry = self
            .entries
            .iter()
            .find(|entry| entry.metadata().id == id && entry.metadata().version == version)
            .ok_or(CatalogContentError::UnknownEntry)?;
        if !entry.metadata().is_selectable() {
            return Err(CatalogContentError::UnusableEntry);
        }
        Ok(EnvironmentCatalogPin {
            id: id.into(),
            version: version.into(),
            catalog_reference: self.source.reference().to_string(),
            resolved_digest: self.digest.to_string(),
        })
    }

    pub fn entry_for_pin(&self, pin: &EnvironmentCatalogPin) -> Result<&E, CatalogContentError> {
        let expected = self.pin(&pin.id, &pin.version)?;
        if expected != *pin {
            return Err(CatalogContentError::PinMismatch);
        }
        self.entries
            .iter()
            .find(|entry| entry.metadata().id == pin.id && entry.metadata().version == pin.version)
            .ok_or(CatalogContentError::UnknownEntry)
    }
}

#[cfg(test)]
mod tests;
