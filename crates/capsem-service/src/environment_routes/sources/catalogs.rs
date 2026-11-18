//! Native catalog selection: current source policy, verified OCI bytes and
//! monotonic Logger observations. Cached content alone grants no authority.
use super::*;
use capsem_assets::catalogs::ValidatedCatalogSource;
use capsem_config::catalogs::{CatalogEntryStatus, RuleCatalogEntry, SkillCatalogEntry};
use capsem_core::environment::{effective_catalog_source, CatalogContentEntry, CatalogContentError, CatalogContents};
use capsem_logger::{
    catalog_revocations::{CatalogRevocation, CatalogRevocationKey},
    WriteOp,
};
use std::time::{SystemTime, UNIX_EPOCH};

type Error = EnvironmentResolutionError;
mod runtime;
pub(crate) use runtime::launch_policy;

async fn source(state: &ServiceState, kind: CatalogKind) -> Result<ValidatedCatalogSource, Error> {
    let corporate = state
        .containers
        .environment_catalog_policy()
        .await
        .map_err(|_| Error::Unavailable)?;
    let manifest = state
        .manifest
        .read()
        .map_err(|_| Error::Unavailable)?
        .as_ref()
        .and_then(|manifest| manifest.catalogs.clone())
        .unwrap_or_default();
    effective_catalog_source(&manifest, &corporate, kind)
        .map_err(|_| Error::Refused)?
        .ok_or(Error::Unavailable)
}

pub(super) async fn resolve(
    state: &ServiceState,
    kind: CatalogKind,
    selection: &EnvironmentCatalogSelection,
) -> Result<EnvironmentCatalogPin, Error> {
    match kind {
        CatalogKind::Skills => resolve_typed::<SkillCatalogEntry>(state, kind, selection).await,
        CatalogKind::Rules => resolve_typed::<RuleCatalogEntry>(state, kind, selection).await,
        // Publisher verification for launch catalogs remains a separate decision.
        CatalogKind::Mcp => Err(Error::Unavailable),
    }
}

async fn resolve_typed<E: CatalogContentEntry>(
    state: &ServiceState,
    kind: CatalogKind,
    selection: &EnvironmentCatalogSelection,
) -> Result<EnvironmentCatalogPin, Error> {
    let selected = source(state, kind).await?;
    let puller = state
        .containers
        .environment_catalog_puller()
        .map_err(|_| Error::Unavailable)?;
    let parent = staging(state).await?;
    let contents = CatalogContents::<E>::fetch(&puller, &selected, &parent)
        .await
        .map_err(content_error)?;
    observe(state, &contents).await?;
    let pin = contents.pin(&selection.id, &selection.version).map_err(content_error)?;
    require_current(state, kind, &pin).await?;
    Ok(pin)
}

pub(super) async fn admit(state: &ServiceState, kind: CatalogKind, pin: &EnvironmentCatalogPin) -> Result<(), Error> {
    match kind {
        CatalogKind::Skills => admit_typed::<SkillCatalogEntry>(state, kind, pin).await,
        CatalogKind::Rules => admit_typed::<RuleCatalogEntry>(state, kind, pin).await,
        CatalogKind::Mcp => Err(Error::Unavailable),
    }
}

async fn admit_typed<E: CatalogContentEntry>(
    state: &ServiceState,
    kind: CatalogKind,
    pin: &EnvironmentCatalogPin,
) -> Result<(), Error> {
    let selected = require_current(state, kind, pin).await?;
    let puller = state
        .containers
        .environment_catalog_puller()
        .map_err(|_| Error::Unavailable)?;
    let parent = staging(state).await?;
    // Retained pins never resolve the moving tag or silently fetch missing bytes.
    CatalogContents::<E>::fetch_cached(&puller, &selected, pin, &parent)
        .await
        .map_err(content_error)?;
    require_current(state, kind, pin).await?;
    Ok(())
}

async fn require_current(
    state: &ServiceState,
    kind: CatalogKind,
    pin: &EnvironmentCatalogPin,
) -> Result<ValidatedCatalogSource, Error> {
    let selected = source(state, kind).await?;
    if selected.reference().to_string() != pin.catalog_reference {
        return Err(Error::Refused);
    }
    let key = CatalogRevocationKey {
        kind,
        source_repository: selected.identity().repository().to_string(),
        entry_id: pin.id.clone(),
        version: pin.version.clone(),
    };
    if state
        .host_ledger
        .catalog_revocation(key)
        .await
        .map_err(|_| Error::Unavailable)?
        .is_some()
    {
        return Err(Error::UnusableCatalogEntry);
    }
    Ok(selected)
}

async fn observe<E: CatalogContentEntry>(state: &ServiceState, contents: &CatalogContents<E>) -> Result<(), Error> {
    let observed_at_unix_ms = i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| Error::Unavailable)?
            .as_millis(),
    )
    .map_err(|_| Error::Unavailable)?;
    let mut wrote = false;
    for entry in contents.entries() {
        let metadata = entry.metadata();
        if metadata.status != CatalogEntryStatus::Revoked {
            continue;
        }
        state
            .host_ledger
            .write(WriteOp::ObserveCatalogRevocation(CatalogRevocation {
                key: CatalogRevocationKey {
                    kind: contents.source().kind(),
                    source_repository: contents.source().identity().repository().to_string(),
                    entry_id: metadata.id.clone(),
                    version: metadata.version.clone(),
                },
                catalog_digest: contents.digest().to_string(),
                observed_at_unix_ms,
            }))
            .await
            .map_err(|_| Error::Unavailable)?;
        wrote = true;
    }
    if wrote {
        state.host_ledger.flush().await.map_err(|_| Error::Unavailable)?;
    }
    Ok(())
}

async fn staging(state: &ServiceState) -> Result<std::path::PathBuf, Error> {
    let parent = state.run_dir.join("environment-catalogs");
    let prepared = parent.clone();
    tokio::task::spawn_blocking(move || capsem_foundation::unix::fs::ensure_private_dir(&prepared))
        .await
        .map_err(|_| Error::Unavailable)?
        .map_err(|_| Error::Unavailable)?;
    Ok(parent)
}

fn content_error(error: CatalogContentError) -> Error {
    match error {
        CatalogContentError::Unavailable | CatalogContentError::WorkerFailed => Error::Unavailable,
        CatalogContentError::UnusableEntry => Error::UnusableCatalogEntry,
        CatalogContentError::UnknownEntry => Error::InvalidIntent,
        _ => Error::InvalidPin,
    }
}
