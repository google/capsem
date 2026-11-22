//! Read the boot selection acknowledged by one live sandbox generation.
use super::*;
use crate::owner_connection::OwnerConnection;
use capsem_api::environments::{VmSkillDetailResponse, VmSkillListResponse};
use capsem_logger::{
    db::{EnvironmentReferenceQuery, EnvironmentReferenceRecord},
    events::environment::EnvironmentSandboxReferenceKind,
};

pub(crate) async fn vm_list(
    actor: AuthenticatedActor,
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    Query(query): Query<EnvironmentSkillListQuery>,
) -> Result<Json<VmSkillListResponse>, AppError> {
    let owner = actor.session_owner(&state, &id, AgentOperation::Inspect)?;
    let page = SkillPage::parse(&query)?;
    let selection = applied_selection(&actor, &state, &owner, page.revision_id).await?;
    let listed = match selection {
        Some((revision, pins)) => Some(read_page(&state, revision, &pins, page).await),
        None => None,
    };
    verify_owner(&actor, &state, &owner)?;
    let (revision, skills, discovery_fresh, next_cursor) = listed.map_or_else(
        || (None, Vec::new(), false, None),
        |page| (Some(page.revision), page.skills, page.discovery_fresh, page.next_cursor),
    );
    Ok(Json(VmSkillListResponse {
        vm_id: id,
        owner_generation: owner.generation().to_string(),
        revision,
        skills,
        discovery_fresh,
        next_cursor,
    }))
}

pub(crate) async fn vm_info(
    actor: AuthenticatedActor,
    State(state): State<Arc<ServiceState>>,
    Path((id, skill_id)): Path<(String, String)>,
    Query(query): Query<EnvironmentPolicyListQuery>,
) -> Result<Json<VmSkillDetailResponse>, AppError> {
    let owner = actor.session_owner(&state, &id, AgentOperation::Inspect)?;
    if !valid_skill_id(&skill_id) {
        return Err(invalid());
    }
    let (revision, pins) = applied_selection(&actor, &state, &owner, query.revision_id.as_deref())
        .await?
        .ok_or_else(not_selected)?;
    let skill = read_info(&state, revision, pins, &skill_id).await;
    verify_owner(&actor, &state, &owner)?;
    Ok(Json(VmSkillDetailResponse {
        vm_id: id,
        owner_generation: owner.generation().to_string(),
        skill: skill?,
    }))
}

async fn applied_selection(
    actor: &AuthenticatedActor,
    state: &ServiceState,
    owner: &OwnerConnection,
    expected: Option<&str>,
) -> Result<Option<(EnvironmentRevisionReference, Vec<EnvironmentCatalogPin>)>, AppError> {
    verify_owner(actor, state, owner)?;
    let boot = {
        let instances = state.instances.lock().unwrap();
        let instance = instances
            .get(owner.id())
            .filter(|instance| instance.generation == owner.generation())
            .ok_or_else(owner_changed)?;
        match &instance.environment {
            Some(binding) if binding.acknowledged => Some(binding.boot.clone()),
            Some(_) => {
                return Err(AppError::new(
                    StatusCode::CONFLICT,
                    "sandbox environment has yet to acknowledge its boot revision".into(),
                ))
            }
            None => None,
        }
    };
    let Some(boot) = boot else {
        return if expected.is_some() {
            Err(revision_changed())
        } else {
            Ok(None)
        };
    };
    if let Some(revision_id) = expected {
        EnvironmentRevisionReference {
            environment_id: boot.environment_id.clone(),
            revision_id: revision_id.into(),
        }
        .validate()
        .map_err(|_| invalid())?;
        if revision_id != boot.revision_id {
            return Err(revision_changed());
        }
    }
    let receipt = state
        .host_ledger
        .environment_references(EnvironmentReferenceQuery::Sandbox {
            session_id: owner.id().into(),
            generation: owner.generation().to_string(),
        })
        .await;
    verify_owner(actor, state, owner)?;
    if !receipt.map_err(unavailable)?.iter().any(|item| {
        matches!(item, EnvironmentReferenceRecord::Sandbox { dimension: EnvironmentSandboxReferenceKind::Boot, revision } if revision == &boot)
    }) {
        return Err(unavailable("sandbox skill view has no matching boot receipt"));
    }
    let snapshot = state
        .environments
        .load_revision(&boot.environment_id, &boot.revision_id)
        .await;
    verify_owner(actor, state, owner)?;
    let revision = EnvironmentRevisionReference {
        environment_id: boot.environment_id,
        revision_id: boot.revision_id,
    };
    Ok(Some((
        revision,
        snapshot.map_err(unavailable)?.specification.skills.selected,
    )))
}

fn verify_owner(actor: &AuthenticatedActor, state: &ServiceState, owner: &OwnerConnection) -> Result<(), AppError> {
    actor.check_session(state, owner, AgentOperation::Inspect)?;
    owner.validate(state, false).map_err(|_| owner_changed())
}

fn owner_changed() -> AppError {
    AppError::new(
        StatusCode::CONFLICT,
        "sandbox generation changed during skill inspection".into(),
    )
}

fn revision_changed() -> AppError {
    AppError::new(
        StatusCode::CONFLICT,
        "sandbox boot revision does not match the requested revision".into(),
    )
}
