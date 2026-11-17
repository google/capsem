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
    LedgerFailure,
    PublicationFailure,
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
            Self::WorkerFailed => "environment operation worker failed",
            Self::LedgerFailure => "environment ledger durability is unconfirmed; reconcile before retrying",
            Self::PublicationFailure => "environment file publication is unconfirmed; reconcile before retrying",
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
        let parent = root
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        capsem_foundation::unix::fs::durable_sync_directory(parent)
            .map_err(|_| EnvironmentFileError::PublicationFailure)?;
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
        let expected = current_revision(ledger, environment_id)
            .await?
            .ok_or(EnvironmentFileError::Unavailable)?;
        let document = self
            .read_existing(environment_id)
            .await?
            .ok_or(EnvironmentFileError::Unavailable)?;
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
        if current_revision(ledger, environment_id).await?.as_deref() != Some(expected.as_str()) {
            return Err(EnvironmentFileError::Changed);
        }
        Ok(CurrentEnvironment { document, revision })
    }

    pub(crate) async fn read_existing(&self, id: &str) -> Result<Option<EnvironmentDocument>, EnvironmentFileError> {
        validate_id(id)?;
        let root = Arc::clone(&self.root);
        let id = id.to_owned();
        tokio::task::spawn_blocking(move || read_document(&root, &id))
            .await
            .map_err(|_| EnvironmentFileError::WorkerFailed)?
    }
    pub(crate) async fn publish_serialized(&self, id: String, content: String) -> Result<(), EnvironmentFileError> {
        validate_id(&id)?;
        if content.len() > capsem_proto::ENVIRONMENT_MAX_SPEC_BYTES {
            return Err(EnvironmentFileError::TooLarge);
        }
        let root = Arc::clone(&self.root);
        tokio::task::spawn_blocking(move || {
            let directory = root
                .descend_or_create(OsStr::new(&id), 0o700)
                .map_err(|_| EnvironmentFileError::UnsafeFile)?;
            capsem_foundation::unix::fs::atomic_write_private_at(
                &directory,
                OsStr::new("environment.toml"),
                content.as_bytes(),
            )
            .map_err(|_| EnvironmentFileError::PublicationFailure)?;
            root.sync().map_err(|_| EnvironmentFileError::PublicationFailure)
        })
        .await
        .map_err(|_| EnvironmentFileError::WorkerFailed)?
    }
    pub(crate) async fn sync_existing(&self, id: &str) -> Result<(), EnvironmentFileError> {
        validate_id(id)?;
        let root = Arc::clone(&self.root);
        let id = id.to_owned();
        tokio::task::spawn_blocking(move || {
            let directory = root
                .descend(OsStr::new(&id))
                .map_err(|_| EnvironmentFileError::UnsafeFile)?;
            directory
                .validate_private()
                .map_err(|_| EnvironmentFileError::UnsafeFile)?;
            let file = directory
                .open_single_link_file(OsStr::new("environment.toml"), ContainedOpenOptions::read_only())
                .map_err(|_| EnvironmentFileError::UnsafeFile)?;
            file.sync_all()
                .and_then(|()| directory.sync())
                .and_then(|()| root.sync())
                .map_err(|_| EnvironmentFileError::PublicationFailure)
        })
        .await
        .map_err(|_| EnvironmentFileError::WorkerFailed)?
    }
    #[cfg(test)]
    pub(crate) async fn publish_document(&self, document: EnvironmentDocument) -> Result<(), EnvironmentFileError> {
        let content = document.to_toml().map_err(|_| EnvironmentFileError::InvalidDocument)?;
        self.publish_serialized(document.identity.id, content).await
    }
}

pub(crate) fn validate_id(id: &str) -> Result<(), EnvironmentFileError> {
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
pub(crate) async fn current_revision(ledger: &DbHandle, id: &str) -> Result<Option<String>, EnvironmentFileError> {
    let references = ledger
        .environment_references(EnvironmentReferenceQuery::CurrentEnvironment {
            environment_id: id.into(),
        })
        .await
        .map_err(|_| EnvironmentFileError::Unavailable)?;
    match references.into_iter().next() {
        Some(EnvironmentReferenceRecord::CurrentEnvironment { revision }) if revision.environment_id == id => {
            Ok(Some(revision.revision_id))
        }
        None => Ok(None),
        _ => Err(EnvironmentFileError::Unverified),
    }
}
fn read_document(root: &ContainedDir, id: &str) -> Result<Option<EnvironmentDocument>, EnvironmentFileError> {
    let directory = match root.descend(OsStr::new(id)) {
        Ok(directory) => directory,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(EnvironmentFileError::UnsafeFile),
    };
    directory
        .validate_private()
        .map_err(|_| EnvironmentFileError::UnsafeFile)?;
    let file = match directory.open_single_link_file(OsStr::new("environment.toml"), ContainedOpenOptions::read_only())
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(EnvironmentFileError::UnsafeFile),
    };
    let mut bytes = Vec::new();
    file.take(capsem_proto::ENVIRONMENT_MAX_SPEC_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| EnvironmentFileError::Unavailable)?;
    if bytes.len() > capsem_proto::ENVIRONMENT_MAX_SPEC_BYTES {
        return Err(EnvironmentFileError::TooLarge);
    }
    let source = std::str::from_utf8(&bytes).map_err(|_| EnvironmentFileError::InvalidDocument)?;
    EnvironmentDocument::from_toml(source)
        .map(Some)
        .map_err(|_| EnvironmentFileError::InvalidDocument)
}

#[cfg(test)]
mod tests;
