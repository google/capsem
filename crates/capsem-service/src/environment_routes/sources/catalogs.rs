//! Native catalog selection: current source policy, verified OCI bytes and
//! monotonic Logger observations. Cached content alone grants no authority.
use super::*;
use capsem_api::environments::{CatalogAvailability, CatalogRefreshError, CatalogStatus};
use capsem_assets::{catalogs::ValidatedCatalogSource, oci::Puller};
use capsem_config::catalogs::{CatalogEntryStatus, CatalogSources, RuleCatalogEntry, SkillCatalogEntry};
use capsem_core::environment::{CatalogContentEntry, CatalogContentError, CatalogContents};
use capsem_logger::{
    catalog_revocations::{CatalogRevocation, CatalogRevocationKey},
    WriteOp,
};
use std::time::{SystemTime, UNIX_EPOCH};

type Error = EnvironmentResolutionError;
mod runtime;
pub(in crate::environment_routes) mod skills;
mod status;
pub(crate) use runtime::launch_policy;
pub(crate) use status::Cache as CatalogStatusCache;

pub(crate) async fn refresh_catalog_sources(state: &ServiceState) {
    check_sources(state).await;
}

struct Selection {
    status: CatalogStatus,
    validated: Result<ValidatedCatalogSource, Error>,
}

async fn check_sources(state: &ServiceState) -> [Selection; CatalogKind::ALL.len()] {
    let ticket = state.catalog_status.begin();
    let corporate = state.containers.environment_catalog_policy().await;
    let manifest = state.manifest.read();
    let selections = match (corporate, manifest) {
        (Ok(corporate), Ok(manifest)) => {
            let sources = manifest.as_ref().and_then(|manifest| manifest.catalogs.as_ref());
            let empty = CatalogSources::default();
            CatalogKind::ALL.map(|kind| {
                select_source(
                    kind,
                    sources.unwrap_or(&empty),
                    corporate.catalogs.as_ref().unwrap_or(&empty),
                )
            })
        }
        _ => CatalogKind::ALL.map(|kind| Selection {
            status: status::unavailable(kind, CatalogRefreshError::PolicyUnavailable),
            validated: Err(Error::Unavailable),
        }),
    };
    state
        .catalog_status
        .observe_policy(ticket, selections.each_ref().map(|item| item.status.clone()));
    selections
}

fn select_source(kind: CatalogKind, manifest: &CatalogSources, corporate: &CatalogSources) -> Selection {
    let Some(chosen) = manifest.effective_source(corporate, kind) else {
        return Selection {
            status: status::unavailable(kind, CatalogRefreshError::MissingSource),
            validated: Err(Error::Unavailable),
        };
    };
    let mut status = status::unavailable(kind, CatalogRefreshError::InvalidSource);
    status.origin = Some(chosen.origin);
    let validated = ValidatedCatalogSource::new(kind, chosen).map_err(|_| Error::Refused);
    if let Ok(source) = &validated {
        status.reference = Some(source.reference().to_string());
        if kind == CatalogKind::Mcp {
            status.error = Some(CatalogRefreshError::Unsupported);
        } else {
            status.availability = CatalogAvailability::Unverified;
            status.error = None;
        }
    }
    Selection { status, validated }
}

pub(in crate::environment_routes) async fn source(
    state: &ServiceState,
    kind: CatalogKind,
) -> Result<ValidatedCatalogSource, Error> {
    check_sources(state)
        .await
        .into_iter()
        .find(|entry| entry.status.kind == kind)
        .ok_or(Error::Unavailable)?
        .validated
}

pub(super) async fn resolve(
    state: &ServiceState,
    kind: CatalogKind,
    selection: &EnvironmentCatalogSelection,
) -> Result<EnvironmentCatalogPin, Error> {
    match kind {
        CatalogKind::Skills => resolve_typed::<SkillCatalogEntry>(state, kind, selection).await,
        CatalogKind::Rules => resolve_typed::<RuleCatalogEntry>(state, kind, selection).await,
        // MCP sources are digest-pinned at OCI admission; typed MCP content
        // verification and selection are pending.
        CatalogKind::Mcp => Err(Error::Unavailable),
    }
}

async fn resolve_typed<E: CatalogContentEntry>(
    state: &ServiceState,
    kind: CatalogKind,
    selection: &EnvironmentCatalogSelection,
) -> Result<EnvironmentCatalogPin, Error> {
    let contents = fetch_current::<E>(state, kind).await?;
    let pin = contents.pin(&selection.id, &selection.version).map_err(content_error)?;
    require_current(state, kind, &pin).await?;
    Ok(pin)
}

pub(in crate::environment_routes) async fn fetch_current<E: CatalogContentEntry>(
    state: &ServiceState,
    kind: CatalogKind,
) -> Result<CatalogContents<E>, Error> {
    let selected = source(state, kind).await?;
    let workspace = workspace_for_refresh(state, &selected).await?;
    refresh::<E>(state, &selected, &workspace)
        .await?
        .ok_or(Error::Unavailable)
}

struct Workspace {
    puller: Puller,
    parent: std::path::PathBuf,
}

async fn workspace_for_refresh(state: &ServiceState, selected: &ValidatedCatalogSource) -> Result<Workspace, Error> {
    let ticket = state.catalog_status.begin();
    let result = async {
        let puller = state
            .containers
            .environment_catalog_puller()
            .map_err(|_| Error::Unavailable)?;
        let parent = staging(state).await?;
        Ok(Workspace { puller, parent })
    }
    .await;
    if result.is_err() {
        state
            .catalog_status
            .observe_refresh(ticket, selected, Err(CatalogRefreshError::LocalUnavailable));
    }
    result
}

/// An unavailable registry permits the existing exact retained-skill path.
/// Invalid content and failures to record revocations still fail closed.
async fn refresh<E: CatalogContentEntry>(
    state: &ServiceState,
    selected: &ValidatedCatalogSource,
    workspace: &Workspace,
) -> Result<Option<CatalogContents<E>>, Error> {
    let ticket = state.catalog_status.begin();
    let (result, failure) = match CatalogContents::<E>::fetch(&workspace.puller, selected, &workspace.parent).await {
        Ok(contents) => match observe(state, &contents).await {
            Ok(()) => (Ok(Some(contents)), None),
            Err(error) => (Err(error), Some(CatalogRefreshError::LocalUnavailable)),
        },
        Err(CatalogContentError::Unavailable) => (Ok(None), Some(CatalogRefreshError::RegistryUnavailable)),
        Err(CatalogContentError::WorkerFailed) => {
            (Err(Error::Unavailable), Some(CatalogRefreshError::LocalUnavailable))
        }
        Err(error) => (Err(content_error(error)), Some(CatalogRefreshError::VerificationFailed)),
    };
    if let Some(error) = failure {
        state.catalog_status.observe_refresh(ticket, selected, Err(error));
    } else if let Ok(Some(contents)) = &result {
        state.catalog_status.observe_refresh(
            ticket,
            selected,
            Ok((contents.digest().as_str(), crate::policy_mutation::unix_timestamp_ms())),
        );
    }
    result
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

pub(in crate::environment_routes) async fn require_current(
    state: &ServiceState,
    kind: CatalogKind,
    pin: &EnvironmentCatalogPin,
) -> Result<ValidatedCatalogSource, Error> {
    let selected = source(state, kind).await?;
    if selected.reference().to_string() != pin.catalog_reference {
        return Err(Error::Refused);
    }
    if known_revoked(state, &selected, &pin.id, &pin.version).await? {
        return Err(Error::UnusableCatalogEntry);
    }
    Ok(selected)
}

pub(in crate::environment_routes) async fn known_revoked(
    state: &ServiceState,
    source: &ValidatedCatalogSource,
    id: &str,
    version: &str,
) -> Result<bool, Error> {
    let key = CatalogRevocationKey {
        kind: source.kind(),
        source_repository: source.identity().repository().to_string(),
        entry_id: id.to_owned(),
        version: version.to_owned(),
    };
    state
        .host_ledger
        .catalog_revocation(key)
        .await
        .map(|revocation| revocation.is_some())
        .map_err(|_| Error::Unavailable)
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

pub(in crate::environment_routes) fn content_error(error: CatalogContentError) -> Error {
    match error {
        CatalogContentError::Unavailable | CatalogContentError::WorkerFailed => Error::Unavailable,
        CatalogContentError::UnusableEntry => Error::UnusableCatalogEntry,
        CatalogContentError::UnknownEntry => Error::InvalidIntent,
        _ => Error::InvalidPin,
    }
}
