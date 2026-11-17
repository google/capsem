//! Canonical current specifications are service-owned; history remains logger-owned.
use capsem_config::environment::EnvironmentDocument;
use capsem_foundation::unix::contained::{ContainedDir, ContainedOpenOptions};
use capsem_logger::{
    db::{EnvironmentReferenceQuery, EnvironmentReferenceRecord, EnvironmentRevisionQuery, EnvironmentRevisionRecord},
    events::EnvironmentRevisionState,
    DbHandle,
};
use std::{ffi::OsStr, io::Read, path::Path, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvironmentFileError {
    InvalidIdentity,
    Unavailable,
    UnsafeFile,
    TooLarge,
    InvalidDocument,
    RecoveryRequired,
    Unverified,
    Changed,
    WorkerFailed,
}
impl std::fmt::Display for EnvironmentFileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidIdentity => "invalid environment identity",
            Self::Unavailable => "current environment or required ledger history is unavailable",
            Self::UnsafeFile => "environment file is not a contained regular private-directory entry",
            Self::TooLarge => "environment file exceeds its byte limit",
            Self::InvalidDocument => "invalid canonical environment document",
            Self::RecoveryRequired => "environment revision recovery is required",
            Self::Unverified => "canonical environment does not match its committed ledger revision",
            Self::Changed => "current environment changed while loading",
            Self::WorkerFailed => "environment file reader worker failed",
        })
    }
}
impl std::error::Error for EnvironmentFileError {}

/// A verified current view, never a grant of caller or mutation authority.
/// The caller still owns authentication and per-environment operation admission.
#[derive(Debug)]
pub struct CurrentEnvironment {
    pub document: EnvironmentDocument,
    pub revision: EnvironmentRevisionRecord,
}

/// Holds the chosen private directory's descriptor, without a projection cache.
pub struct EnvironmentFiles {
    root: Arc<ContainedDir>,
}
impl EnvironmentFiles {
    /// Blocking setup; invoke on the service blocking pool during initialization.
    pub fn open(root: &Path) -> Result<Self, EnvironmentFileError> {
        capsem_foundation::unix::fs::ensure_private_dir(root).map_err(|_| EnvironmentFileError::UnsafeFile)?;
        let root = ContainedDir::open_root(root).map_err(|_| EnvironmentFileError::UnsafeFile)?;
        root.validate_private().map_err(|_| EnvironmentFileError::UnsafeFile)?;
        Ok(Self { root: Arc::new(root) })
    }

    pub async fn load_current(
        &self,
        ledger: &DbHandle,
        environment_id: &str,
    ) -> Result<CurrentEnvironment, EnvironmentFileError> {
        validate_id(environment_id)?;
        let pending = ledger
            .environment_revisions(EnvironmentRevisionQuery::Page {
                environment_id: Some(environment_id.into()),
                state: Some(EnvironmentRevisionState::Committing),
                after_id: 0,
                limit: 1,
            })
            .await
            .map_err(|_| EnvironmentFileError::Unavailable)?;
        if !pending.is_empty() {
            return Err(EnvironmentFileError::RecoveryRequired);
        }
        let expected = current_revision(ledger, environment_id).await?;
        let root = Arc::clone(&self.root);
        let id = environment_id.to_owned();
        let document = tokio::task::spawn_blocking(move || read_document(&root, &id))
            .await
            .map_err(|_| EnvironmentFileError::WorkerFailed)??;
        if document.identity.id != environment_id || document.identity.revision_id != expected {
            return Err(EnvironmentFileError::Unverified);
        }
        let snapshot = ledger
            .environment_revision_snapshot(environment_id, &expected)
            .await
            .map_err(|_| EnvironmentFileError::Unavailable)?
            .ok_or(EnvironmentFileError::Unavailable)?;
        let revision = snapshot.record;
        if revision.state != EnvironmentRevisionState::Committed
            || revision.environment_id != environment_id
            || revision.revision_id != expected
            || document.identity.parent_environment_id.as_deref()
                != revision.fork_parent.as_ref().map(|p| p.environment_id.as_str())
            || document.identity.parent_revision_id.as_deref()
                != revision.fork_parent.as_ref().map(|p| p.revision_id.as_str())
        {
            return Err(EnvironmentFileError::Unverified);
        }
        if document.specification != *snapshot.specification {
            return Err(EnvironmentFileError::Unverified);
        }
        if current_revision(ledger, environment_id).await? != expected {
            return Err(EnvironmentFileError::Changed);
        }
        Ok(CurrentEnvironment { document, revision })
    }
}

fn validate_id(id: &str) -> Result<(), EnvironmentFileError> {
    if uuid::Uuid::parse_str(id).is_ok_and(|value| {
        value.to_string() == id
            && (1..=8).contains(&value.get_version_num())
            && value.get_variant() == uuid::Variant::RFC4122
    }) {
        Ok(())
    } else {
        Err(EnvironmentFileError::InvalidIdentity)
    }
}
async fn current_revision(ledger: &DbHandle, id: &str) -> Result<String, EnvironmentFileError> {
    let references = ledger
        .environment_references(EnvironmentReferenceQuery::CurrentEnvironment {
            environment_id: id.into(),
        })
        .await
        .map_err(|_| EnvironmentFileError::Unavailable)?;
    match references.into_iter().next() {
        Some(EnvironmentReferenceRecord::CurrentEnvironment { revision }) if revision.environment_id == id => {
            Ok(revision.revision_id)
        }
        _ => Err(EnvironmentFileError::Unavailable),
    }
}
fn read_document(root: &ContainedDir, id: &str) -> Result<EnvironmentDocument, EnvironmentFileError> {
    let directory = root
        .descend(OsStr::new(id))
        .map_err(|_| EnvironmentFileError::UnsafeFile)?;
    directory
        .validate_private()
        .map_err(|_| EnvironmentFileError::UnsafeFile)?;
    let file = directory
        .open_single_link_file(OsStr::new("environment.toml"), ContainedOpenOptions::read_only())
        .map_err(|_| EnvironmentFileError::UnsafeFile)?;
    let mut bytes = Vec::new();
    file.take(capsem_proto::ENVIRONMENT_MAX_SPEC_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| EnvironmentFileError::Unavailable)?;
    if bytes.len() > capsem_proto::ENVIRONMENT_MAX_SPEC_BYTES {
        return Err(EnvironmentFileError::TooLarge);
    }
    let source = std::str::from_utf8(&bytes).map_err(|_| EnvironmentFileError::InvalidDocument)?;
    EnvironmentDocument::from_toml(source).map_err(|_| EnvironmentFileError::InvalidDocument)
}

#[cfg(test)]
mod tests;
