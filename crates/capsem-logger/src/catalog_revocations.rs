//! Monotonic host observations, independent of catalog refresh and byte caches.
use capsem_config::catalogs::CatalogKind;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogRevocationKey {
    pub kind: CatalogKind,
    /// Canonical registry/repository, without a moving tag or manifest digest.
    pub source_repository: String,
    pub entry_id: String,
    pub version: String,
}
impl CatalogRevocationKey {
    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        let source = &self.source_repository;
        let repository = source.split_once('/');
        if source.len() > capsem_proto::ENVIRONMENT_MAX_CATALOG_REFERENCE_BYTES
            || !source
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || b".-_/:[]".contains(&byte))
            || source.contains("//")
            || source.ends_with('/')
            || !repository.is_some_and(|(host, path)| !host.is_empty() && !path.is_empty() && !path.contains(':'))
            || self.entry_id.len() > capsem_proto::CATALOG_MAX_IDENTIFIER_BYTES
            || capsem_config::validate_identifier_shape(&self.entry_id).is_err()
            || self.version.is_empty()
            || self.version.len() > capsem_proto::CATALOG_MAX_VERSION_BYTES
            || self.version.chars().any(|ch| ch.is_control() || ch.is_whitespace())
        {
            return Err("invalid catalog revocation identity");
        }
        Ok(())
    }
}

/// The first verified catalog in which the host observed a revoked version.
/// Later observations can neither replace this provenance nor remove the row.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogRevocation {
    pub key: CatalogRevocationKey,
    pub catalog_digest: String,
    pub observed_at_unix_ms: i64,
}
impl CatalogRevocation {
    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        self.key.validate()?;
        let digest = self.catalog_digest.strip_prefix("sha256:");
        if !digest.is_some_and(|hex| {
            hex.len() == 64
                && hex
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        }) || self.observed_at_unix_ms < 0
        {
            return Err("invalid catalog revocation provenance");
        }
        Ok(())
    }
}
