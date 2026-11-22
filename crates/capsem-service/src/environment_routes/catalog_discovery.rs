//! Shared browse operations for OCI catalogs with verified, typed content.
use super::*;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use capsem_api::environments::{CatalogEntryQuery, CatalogListQuery, CatalogSnapshot, EnvironmentCatalogPin};
use capsem_assets::oci::Digest;
use capsem_core::environment::{CatalogContentEntry, CatalogContentError, CatalogContents};
use capsem_proto::{CATALOG_MAX_VERSION_BYTES, ENVIRONMENT_MAX_CATALOG_REFERENCE_BYTES};
use sources::catalogs;

pub(crate) fn permits_discover(actor: &AuthenticatedActor) -> bool {
    actor
        .agent_grant()
        .is_some_and(|grant| grant.authorizes_operation(AgentOperation::Discover, || LeaseClock::now().ok()))
}

fn authorize(actor: &AuthenticatedActor) -> Result<(), AppError> {
    if actor.agent_grant().is_none() {
        return actor.require_operator();
    }
    if permits_discover(actor) {
        return Ok(());
    }
    Err(AppError::new(
        StatusCode::FORBIDDEN,
        "catalog discovery scope required".into(),
    ))
}

pub(super) struct Page<T> {
    pub catalog: CatalogSnapshot,
    pub entries: Vec<T>,
    pub next_cursor: Option<String>,
}

pub(super) async fn list<E: CatalogContentEntry, T>(
    actor: &AuthenticatedActor,
    state: &ServiceState,
    query: CatalogListQuery,
    project: impl Fn(&E) -> Result<T, AppError>,
) -> Result<Page<T>, AppError> {
    authorize(actor)?;
    let (search, limit) = page_options(query.search.as_deref(), query.limit)?;
    let cursor = query.after.as_deref().map(Cursor::parse).transpose()?;
    let result = read_page(state, &search, limit, cursor, project).await;
    authorize(actor)?;
    result
}

async fn read_page<E: CatalogContentEntry, T>(
    state: &ServiceState,
    search: &str,
    limit: usize,
    cursor: Option<Cursor<'_>>,
    project: impl Fn(&E) -> Result<T, AppError>,
) -> Result<Page<T>, AppError> {
    let contents = catalogs::fetch_current::<E>(state, E::KIND)
        .await
        .map_err(resolution_error)?;
    let catalog = snapshot(&contents);
    let after = match &cursor {
        Some(cursor) if cursor.snapshot != catalog.snapshot => return Err(stale_snapshot()),
        Some(cursor) => (cursor.id, cursor.version.as_str()),
        None => ("", ""),
    };
    let mut selected: Vec<_> = contents
        .entries()
        .iter()
        .filter(|entry| {
            entry.metadata().is_selectable() && key::<E>(entry) > after && metadata_matches(entry.metadata(), search)
        })
        .collect();
    selected.sort_unstable_by(|left, right| key::<E>(left).cmp(&key::<E>(right)));
    let mut entries = Vec::with_capacity(limit);
    let mut last = None;
    let mut next_cursor = None;
    for entry in selected {
        let (id, version) = key(entry);
        if catalogs::known_revoked(state, contents.source(), id, version)
            .await
            .map_err(resolution_error)?
        {
            continue;
        }
        if entries.len() == limit {
            next_cursor = last.map(|(id, version)| Cursor::encode(&catalog.snapshot, id, version));
            break;
        }
        entries.push(project(entry)?);
        last = Some((id, version));
    }
    let source = catalogs::source(state, E::KIND).await.map_err(resolution_error)?;
    if source.reference() != contents.source().reference()
        || source.schema_version() != contents.source().schema_version()
    {
        return Err(stale_snapshot());
    }
    Ok(Page {
        catalog,
        entries,
        next_cursor,
    })
}

fn key<E: CatalogContentEntry>(entry: &E) -> (&str, &str) {
    (entry.metadata().id.as_str(), entry.metadata().version.as_str())
}

pub(super) async fn info<E: CatalogContentEntry, T>(
    actor: &AuthenticatedActor,
    state: &ServiceState,
    id: &str,
    query: CatalogEntryQuery,
    project: impl FnOnce(EnvironmentCatalogPin, &E) -> Result<T, AppError>,
) -> Result<T, AppError> {
    authorize(actor)?;
    if !valid_id(id) || !valid_version(&query.version) {
        return Err(invalid());
    }
    validate_snapshot(&query.snapshot)?;
    let result = read_info(state, id, &query, project).await;
    authorize(actor)?;
    result
}

async fn read_info<E: CatalogContentEntry, T>(
    state: &ServiceState,
    id: &str,
    query: &CatalogEntryQuery,
    project: impl FnOnce(EnvironmentCatalogPin, &E) -> Result<T, AppError>,
) -> Result<T, AppError> {
    let contents = catalogs::fetch_current::<E>(state, E::KIND)
        .await
        .map_err(resolution_error)?;
    if snapshot(&contents).snapshot != query.snapshot {
        return Err(stale_snapshot());
    }
    let selection = contents.pin(id, &query.version).map_err(|error| match error {
        CatalogContentError::UnknownEntry => AppError::new(
            StatusCode::NOT_FOUND,
            "entry version is absent from this catalog".into(),
        ),
        error => resolution_error(catalogs::content_error(error)),
    })?;
    let entry = contents
        .entry_for_pin(&selection)
        .map_err(|error| resolution_error(catalogs::content_error(error)))?;
    let current = catalogs::require_current(state, E::KIND, &selection)
        .await
        .map_err(resolution_error)?;
    if current.schema_version() != contents.source().schema_version() {
        return Err(stale_snapshot());
    }
    project(selection, entry)
}

pub(super) fn resolution_error(error: EnvironmentResolutionError) -> AppError {
    mutation_error(EnvironmentMutationError::Resolution(error))
}

pub(super) fn valid_id(id: &str) -> bool {
    id.len() <= capsem_proto::CATALOG_MAX_IDENTIFIER_BYTES && capsem_config::validate_identifier_shape(id).is_ok()
}

fn invalid() -> AppError {
    AppError::new(
        StatusCode::BAD_REQUEST,
        "invalid catalog entry or pagination query".into(),
    )
}

fn stale_snapshot() -> AppError {
    AppError::new(
        StatusCode::CONFLICT,
        "catalog source or digest changed, list the catalog again".into(),
    )
}

pub(super) fn page_options(search: Option<&str>, limit: Option<u16>) -> Result<(String, usize), AppError> {
    let search = search.unwrap_or_default();
    let limit = usize::from(limit.unwrap_or(24));
    if search.len() > 64 || search.chars().any(char::is_control) || !(1..=64).contains(&limit) {
        return Err(invalid());
    }
    Ok((search.trim().to_lowercase(), limit))
}

pub(super) fn metadata_matches(metadata: &capsem_config::catalogs::CatalogEntryMetadata, search: &str) -> bool {
    search.is_empty()
        || [
            &metadata.id,
            &metadata.version,
            &metadata.name,
            &metadata.description,
            &metadata.publisher,
        ]
        .into_iter()
        .chain(&metadata.tags)
        .any(|value| value.to_lowercase().contains(search))
}

fn snapshot<E: CatalogContentEntry>(contents: &CatalogContents<E>) -> CatalogSnapshot {
    let reference = contents.source().reference().to_string();
    let digest = contents.digest().to_string();
    let snapshot = URL_SAFE_NO_PAD.encode(format!("{reference}\n{digest}"));
    CatalogSnapshot {
        reference,
        digest,
        snapshot,
    }
}

fn validate_snapshot(snapshot: &str) -> Result<(), AppError> {
    // Base64 of a bounded OCI reference, a separator and a SHA-256 digest.
    const MAX_BYTES: usize = (ENVIRONMENT_MAX_CATALOG_REFERENCE_BYTES + 1 + 71).div_ceil(3) * 4;
    if snapshot.len() > MAX_BYTES {
        return Err(invalid());
    }
    let decoded = URL_SAFE_NO_PAD.decode(snapshot).map_err(|_| invalid())?;
    let text = std::str::from_utf8(&decoded).map_err(|_| invalid())?;
    let (reference, digest) = text.split_once('\n').ok_or_else(invalid)?;
    if reference.is_empty()
        || reference.len() > ENVIRONMENT_MAX_CATALOG_REFERENCE_BYTES
        || Digest::parse(digest).is_err()
    {
        return Err(invalid());
    }
    Ok(())
}

struct Cursor<'a> {
    snapshot: &'a str,
    id: &'a str,
    version: String,
}

impl<'a> Cursor<'a> {
    fn parse(value: &'a str) -> Result<Self, AppError> {
        let (snapshot, seek) = value.split_once('.').ok_or_else(invalid)?;
        let (id, encoded_version) = seek.split_once('.').ok_or_else(invalid)?;
        validate_snapshot(snapshot)?;
        if !valid_id(id) || encoded_version.len() > CATALOG_MAX_VERSION_BYTES.div_ceil(3) * 4 {
            return Err(invalid());
        }
        let decoded = URL_SAFE_NO_PAD.decode(encoded_version).map_err(|_| invalid())?;
        let version = String::from_utf8(decoded).map_err(|_| invalid())?;
        if !valid_version(&version) {
            return Err(invalid());
        }
        Ok(Self { snapshot, id, version })
    }

    fn encode(snapshot: &str, id: &str, version: &str) -> String {
        format!("{snapshot}.{id}.{}", URL_SAFE_NO_PAD.encode(version))
    }
}

fn valid_version(version: &str) -> bool {
    !version.is_empty()
        && version.len() <= CATALOG_MAX_VERSION_BYTES
        && !version.chars().any(|ch| ch.is_control() || ch.is_whitespace())
}
