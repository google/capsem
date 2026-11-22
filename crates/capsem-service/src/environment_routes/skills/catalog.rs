//! Discover skills from the currently configured, verified OCI catalog.
use super::*;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use capsem_api::environments::{
    SkillCatalogDetailQuery, SkillCatalogDetailResponse, SkillCatalogListQuery, SkillCatalogListResponse,
    SkillCatalogSnapshot,
};
use capsem_assets::oci::Digest;
use capsem_config::catalogs::{CatalogKind, SkillCatalogEntry};
use capsem_core::environment::{CatalogContentError, CatalogContents};
use capsem_proto::{CATALOG_MAX_VERSION_BYTES, ENVIRONMENT_MAX_CATALOG_REFERENCE_BYTES};
use sources::catalogs;

const KIND: CatalogKind = CatalogKind::Skills;

pub(crate) fn permits_discover(actor: &AuthenticatedActor) -> bool {
    actor
        .agent_grant()
        .is_some_and(|grant| grant.authorizes_operation(AgentOperation::Discover, || LeaseClock::now().ok()))
}

fn authorize_discover(actor: &AuthenticatedActor) -> Result<(), AppError> {
    if actor.agent_grant().is_none() {
        return actor.require_operator();
    }
    if permits_discover(actor) {
        return Ok(());
    }
    Err(AppError::new(
        StatusCode::FORBIDDEN,
        "skill discovery scope required".into(),
    ))
}

pub(crate) async fn list(
    actor: AuthenticatedActor,
    State(state): State<Arc<ServiceState>>,
    Query(query): Query<SkillCatalogListQuery>,
) -> Result<Json<SkillCatalogListResponse>, AppError> {
    authorize_discover(&actor)?;
    let (search, limit) = page_options(query.search.as_deref(), query.limit)?;
    let cursor = query.after.as_deref().map(Cursor::parse).transpose()?;
    let result = read_page(&state, &search, limit, cursor).await;
    authorize_discover(&actor)?;
    result.map(Json)
}

async fn read_page(
    state: &ServiceState,
    search: &str,
    limit: usize,
    cursor: Option<Cursor<'_>>,
) -> Result<SkillCatalogListResponse, AppError> {
    let contents = catalogs::fetch_current::<SkillCatalogEntry>(state, KIND)
        .await
        .map_err(read_error)?;
    let catalog = snapshot(&contents);
    let after = match &cursor {
        Some(cursor) if cursor.snapshot != catalog.snapshot => return Err(stale_snapshot()),
        Some(cursor) => (cursor.id, cursor.version.as_str()),
        None => ("", ""),
    };
    let mut entries: Vec<_> = contents
        .entries()
        .iter()
        .map(|entry| &entry.metadata)
        .filter(|entry| {
            entry.is_selectable()
                && (entry.id.as_str(), entry.version.as_str()) > after
                && metadata_matches(entry, search)
        })
        .collect();
    entries.sort_unstable_by(|left, right| (&left.id, &left.version).cmp(&(&right.id, &right.version)));
    let mut skills = Vec::with_capacity(limit + 1);
    for entry in entries {
        if !catalogs::known_revoked(state, contents.source(), &entry.id, &entry.version)
            .await
            .map_err(read_error)?
        {
            skills.push(entry.clone());
        }
        if skills.len() > limit {
            break;
        }
    }
    let next_cursor = if skills.len() > limit {
        skills.pop();
        skills
            .last()
            .map(|last| Cursor::encode(&catalog.snapshot, &last.id, &last.version))
    } else {
        None
    };
    let source = catalogs::source(state, KIND).await.map_err(read_error)?;
    if source.reference() != contents.source().reference()
        || source.schema_version() != contents.source().schema_version()
    {
        return Err(stale_snapshot());
    }
    Ok(SkillCatalogListResponse {
        catalog,
        skills,
        next_cursor,
    })
}

pub(crate) async fn info(
    actor: AuthenticatedActor,
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    Query(query): Query<SkillCatalogDetailQuery>,
) -> Result<Json<SkillCatalogDetailResponse>, AppError> {
    authorize_discover(&actor)?;
    if !valid_skill_id(&id) || !valid_version(&query.version) {
        return Err(invalid());
    }
    validate_snapshot(&query.snapshot)?;
    let result = read_info(&state, &id, &query).await;
    authorize_discover(&actor)?;
    result.map(Json)
}

async fn read_info(
    state: &ServiceState,
    id: &str,
    query: &SkillCatalogDetailQuery,
) -> Result<SkillCatalogDetailResponse, AppError> {
    let contents = catalogs::fetch_current::<SkillCatalogEntry>(state, KIND)
        .await
        .map_err(read_error)?;
    if snapshot(&contents).snapshot != query.snapshot {
        return Err(stale_snapshot());
    }
    let selection = contents.pin(id, &query.version).map_err(|error| match error {
        CatalogContentError::UnknownEntry => AppError::new(
            StatusCode::NOT_FOUND,
            "skill version is absent from this catalog".into(),
        ),
        error => read_error(catalogs::content_error(error)),
    })?;
    let entry = contents
        .entry_for_pin(&selection)
        .map_err(|error| read_error(catalogs::content_error(error)))?;
    let current = catalogs::require_current(state, KIND, &selection)
        .await
        .map_err(read_error)?;
    if current.schema_version() != contents.source().schema_version() {
        return Err(stale_snapshot());
    }
    Ok(SkillCatalogDetailResponse {
        selection,
        metadata: entry.metadata.clone(),
        files: skill_files(entry)?,
    })
}

fn snapshot(contents: &CatalogContents<SkillCatalogEntry>) -> SkillCatalogSnapshot {
    let reference = contents.source().reference().to_string();
    let digest = contents.digest().to_string();
    let snapshot = URL_SAFE_NO_PAD.encode(format!("{reference}\n{digest}"));
    SkillCatalogSnapshot {
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
        if !valid_skill_id(id) || encoded_version.len() > CATALOG_MAX_VERSION_BYTES.div_ceil(3) * 4 {
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

fn stale_snapshot() -> AppError {
    AppError::new(
        StatusCode::CONFLICT,
        "skill catalog source or digest changed, list the catalog again".into(),
    )
}
