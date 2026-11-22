//! Inspect the selected skill catalog without writing files or granting tool access.
use super::*;
use axum::extract::Query;
use capsem_api::environments::{
    EnvironmentCatalogPin, EnvironmentSkillAvailability as Availability, EnvironmentSkillDetailResponse,
    EnvironmentSkillListQuery, EnvironmentSkillListResponse, EnvironmentSkillSummary,
};
use catalog_discovery::{metadata_matches, page_options, valid_id as valid_skill_id};
use sources::catalogs::skills::PinnedSkills;
mod applied;
pub(crate) mod catalog;
pub(crate) use applied::{vm_info, vm_list};

pub(crate) async fn admit_boot_skills(
    state: &ServiceState,
    selected: &[EnvironmentCatalogPin],
) -> Result<(), EnvironmentResolutionError> {
    if selected.is_empty() {
        return Ok(());
    }
    let pins = ordered_pins(selected, "");
    let mut catalog = PinnedSkills::open(state, &pins).await?;
    for pin in pins {
        catalog.entry(state, pin).await?;
    }
    Ok(())
}

pub(crate) async fn list(
    actor: AuthenticatedActor,
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    Query(query): Query<EnvironmentSkillListQuery>,
) -> Result<Json<EnvironmentSkillListResponse>, AppError> {
    let page = SkillPage::parse(&query)?;
    let (revision, specification) = policy_document(
        &actor,
        &state,
        id,
        EnvironmentPolicyListQuery {
            revision_id: page.revision_id.map(str::to_owned),
        },
    )
    .await?;
    let listed = read_page(&state, revision, &specification.skills.selected, page).await;
    authorize(&actor, &listed.revision.environment_id)?;
    Ok(Json(listed))
}

struct SkillPage<'a> {
    revision_id: Option<&'a str>,
    after: &'a str,
    search: String,
    limit: usize,
}

impl<'a> SkillPage<'a> {
    fn parse(query: &'a EnvironmentSkillListQuery) -> Result<Self, AppError> {
        let (search, limit) = page_options(query.search.as_deref(), query.limit)?;
        let cursor = query.after.as_deref().map(parse_cursor).transpose()?;
        let revision_id = match (query.revision_id.as_deref(), cursor) {
            (Some(expected), Some((revision, _))) if expected != revision => return Err(stale_cursor()),
            (None, Some((revision, _))) => Some(revision),
            (expected, _) => expected,
        };
        Ok(Self {
            revision_id,
            after: cursor.map_or("", |(_, id)| id),
            search,
            limit,
        })
    }
}

fn ordered_pins<'a>(selected: &'a [EnvironmentCatalogPin], after: &str) -> Vec<&'a EnvironmentCatalogPin> {
    let mut pins = selected
        .iter()
        .filter(|pin| pin.id.as_str() > after)
        .collect::<Vec<_>>();
    // Process each retained manifest once while keeping at most one retained
    // catalog in memory. Restore ID order when producing the page.
    pins.sort_unstable_by(|left, right| left.resolved_digest.cmp(&right.resolved_digest));
    pins
}

async fn read_page(
    state: &ServiceState,
    revision: EnvironmentRevisionReference,
    selected: &[EnvironmentCatalogPin],
    page: SkillPage<'_>,
) -> EnvironmentSkillListResponse {
    let pins = ordered_pins(selected, page.after);
    let mut loaded = if pins.is_empty() {
        Err(EnvironmentResolutionError::Unavailable)
    } else {
        PinnedSkills::open(state, &pins).await
    };
    let mut skills = Vec::new();
    for selection in pins {
        let result = match &mut loaded {
            Ok(catalog) => catalog
                .entry(state, selection)
                .await
                .map(|entry| entry.metadata.clone()),
            Err(error) => Err(*error),
        };
        let (availability, metadata) = match result {
            Ok(metadata) => (Availability::Available, Some(metadata)),
            Err(EnvironmentResolutionError::UnusableCatalogEntry) => (Availability::Revoked, None),
            Err(_) => (Availability::Unavailable, None),
        };
        let summary = EnvironmentSkillSummary {
            selection: selection.clone(),
            metadata,
            availability,
        };
        if matches_search(&summary, &page.search) {
            skills.push(summary);
        }
    }
    skills.sort_unstable_by(|left, right| left.selection.id.cmp(&right.selection.id));
    let has_next = skills.len() > page.limit;
    skills.truncate(page.limit);
    let next_cursor = has_next
        .then(|| {
            skills
                .last()
                .map(|item| format!("{}:{}", revision.revision_id, item.selection.id))
        })
        .flatten();
    EnvironmentSkillListResponse {
        revision,
        skills,
        discovery_fresh: loaded.is_ok_and(|catalog| catalog.discovery_fresh()),
        next_cursor,
    }
}

pub(crate) async fn info(
    actor: AuthenticatedActor,
    State(state): State<Arc<ServiceState>>,
    Path((id, skill_id)): Path<(String, String)>,
    Query(query): Query<EnvironmentPolicyListQuery>,
) -> Result<Json<EnvironmentSkillDetailResponse>, AppError> {
    if !valid_skill_id(&skill_id) {
        return Err(invalid());
    }
    let (revision, specification) = policy_document(&actor, &state, id, query).await?;
    let result = read_info(&state, revision.clone(), specification.skills.selected, &skill_id).await;
    authorize(&actor, &revision.environment_id)?;
    result.map(Json)
}

async fn read_info(
    state: &ServiceState,
    revision: EnvironmentRevisionReference,
    selected: Vec<EnvironmentCatalogPin>,
    skill_id: &str,
) -> Result<EnvironmentSkillDetailResponse, AppError> {
    let selection = selected
        .into_iter()
        .find(|pin| pin.id == skill_id)
        .ok_or_else(not_selected)?;
    let mut catalog = PinnedSkills::open(state, &[&selection]).await.map_err(read_error)?;
    let discovery_fresh = catalog.discovery_fresh();
    let entry = catalog.entry(state, &selection).await.map_err(read_error)?;
    let files = skill_files(entry)?;
    Ok(EnvironmentSkillDetailResponse {
        revision,
        selection,
        metadata: entry.metadata.clone(),
        discovery_fresh,
        files,
    })
}

fn skill_files(
    entry: &capsem_config::catalogs::SkillCatalogEntry,
) -> Result<std::collections::BTreeMap<String, String>, AppError> {
    let payload = entry
        .payload
        .as_ref()
        .ok_or_else(|| read_error(EnvironmentResolutionError::InvalidPin))?;
    Ok(payload
        .files
        .iter()
        .map(|(path, file)| (path.clone(), file.content.clone()))
        .collect())
}

fn not_selected() -> AppError {
    AppError::new(StatusCode::NOT_FOUND, "skill is not selected in this revision".into())
}

fn read_error(error: EnvironmentResolutionError) -> AppError {
    mutation_error(EnvironmentMutationError::Resolution(error))
}

fn parse_cursor(cursor: &str) -> Result<(&str, &str), AppError> {
    let (revision, id) = cursor.split_once(':').ok_or_else(invalid)?;
    let identity = EnvironmentRevisionReference {
        environment_id: revision.into(),
        revision_id: revision.into(),
    };
    if !valid_skill_id(id) || identity.validate().is_err() {
        return Err(invalid());
    }
    Ok((revision, id))
}

fn invalid() -> AppError {
    AppError::new(StatusCode::BAD_REQUEST, "invalid skill path or pagination query".into())
}

fn stale_cursor() -> AppError {
    AppError::new(
        StatusCode::CONFLICT,
        "skill cursor belongs to another environment revision".into(),
    )
}

fn matches_search(summary: &EnvironmentSkillSummary, search: &str) -> bool {
    search.is_empty()
        || summary.selection.id.to_lowercase().contains(search)
        || summary.selection.version.to_lowercase().contains(search)
        || summary
            .metadata
            .as_ref()
            .is_some_and(|metadata| metadata_matches(metadata, search))
}
