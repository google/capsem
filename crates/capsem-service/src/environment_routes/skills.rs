//! Inspect the selected skill catalog without writing files or granting tool access.
use super::*;
use axum::extract::Query;
use capsem_api::environments::{
    EnvironmentSkillAvailability as Availability, EnvironmentSkillDetailResponse, EnvironmentSkillListQuery,
    EnvironmentSkillListResponse, EnvironmentSkillSummary,
};
use sources::catalogs::skills::PinnedSkills;

pub(crate) async fn list(
    actor: AuthenticatedActor,
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    Query(query): Query<EnvironmentSkillListQuery>,
) -> Result<Json<EnvironmentSkillListResponse>, AppError> {
    let search = query.search.as_deref().unwrap_or_default();
    let limit = usize::from(query.limit.unwrap_or(24));
    if search.len() > 64 || search.chars().any(char::is_control) || !(1..=64).contains(&limit) {
        return Err(invalid());
    }
    let search = search.trim().to_lowercase();
    let cursor = query.after.as_deref().map(parse_cursor).transpose()?;
    let expected = match (query.revision_id, cursor) {
        (Some(expected), Some((revision, _))) if expected != revision => return Err(stale_cursor()),
        (None, Some((revision, _))) => Some(revision.to_owned()),
        (expected, _) => expected,
    };
    let (revision, specification) =
        policy_document(&actor, &state, id, EnvironmentPolicyListQuery { revision_id: expected }).await?;
    let after = cursor.map_or("", |(_, id)| id);
    let mut pins = specification
        .skills
        .selected
        .iter()
        .filter(|pin| pin.id.as_str() > after)
        .collect::<Vec<_>>();
    // Process each retained manifest once while keeping at most one retained
    // catalog in memory. Restore ID order for stable pagination.
    pins.sort_unstable_by(|left, right| left.resolved_digest.cmp(&right.resolved_digest));
    let mut loaded = if pins.is_empty() {
        Err(EnvironmentResolutionError::Unavailable)
    } else {
        PinnedSkills::open(&state, &pins).await
    };
    let mut skills = Vec::new();
    for selection in pins {
        let result = match &mut loaded {
            Ok(catalog) => catalog
                .entry(&state, selection)
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
        if matches_search(&summary, &search) {
            skills.push(summary);
        }
    }
    skills.sort_unstable_by(|left, right| left.selection.id.cmp(&right.selection.id));
    let has_next = skills.len() > limit;
    skills.truncate(limit);
    let next_cursor = has_next
        .then(|| {
            skills
                .last()
                .map(|item| format!("{}:{}", revision.revision_id, item.selection.id))
        })
        .flatten();
    authorize(&actor, &revision.environment_id)?;
    Ok(Json(EnvironmentSkillListResponse {
        revision,
        skills,
        discovery_fresh: loaded.is_ok_and(|catalog| catalog.discovery_fresh()),
        next_cursor,
    }))
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
    let selection = specification
        .skills
        .selected
        .into_iter()
        .find(|pin| pin.id == skill_id)
        .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "skill is not selected in this revision".into()))?;
    let loaded = PinnedSkills::open(&state, &[&selection]).await;
    authorize(&actor, &revision.environment_id)?;
    let mut catalog = loaded.map_err(read_error)?;
    let discovery_fresh = catalog.discovery_fresh();
    let entry = catalog.entry(&state, &selection).await;
    authorize(&actor, &revision.environment_id)?;
    let entry = entry.map_err(read_error)?;
    let payload = entry
        .payload
        .as_ref()
        .ok_or_else(|| read_error(EnvironmentResolutionError::InvalidPin))?;
    let files = payload
        .files
        .iter()
        .map(|(path, file)| (path.clone(), file.content.clone()))
        .collect();
    Ok(Json(EnvironmentSkillDetailResponse {
        revision,
        selection,
        metadata: entry.metadata.clone(),
        discovery_fresh,
        files,
    }))
}

fn read_error(error: EnvironmentResolutionError) -> AppError {
    mutation_error(EnvironmentMutationError::Resolution(error))
}

fn valid_skill_id(id: &str) -> bool {
    id.len() <= capsem_proto::CATALOG_MAX_IDENTIFIER_BYTES && capsem_config::validate_identifier_shape(id).is_ok()
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
    let matches = |text: &str| text.to_lowercase().contains(search);
    search.is_empty()
        || matches(&summary.selection.id)
        || matches(&summary.selection.version)
        || summary.metadata.as_ref().is_some_and(|metadata| {
            matches(&metadata.name)
                || matches(&metadata.description)
                || matches(&metadata.publisher)
                || metadata.tags.iter().any(|tag| matches(tag))
        })
}
