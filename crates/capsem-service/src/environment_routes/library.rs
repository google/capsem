//! Read only the caller's committed environment heads and bounded display metadata.
use super::*;
use axum::extract::Query;
use capsem_api::environments::{
    EnvironmentListItem, EnvironmentListQuery, EnvironmentListResponse, EnvironmentListSummary,
    EnvironmentRevisionReference,
};
use capsem_logger::db::{EnvironmentReferenceRecord, ENVIRONMENT_HEAD_PAGE_MAX};
use std::collections::HashMap;

pub(crate) fn permits_list(actor: &AuthenticatedActor) -> bool {
    actor.agent_grant().is_some_and(|grant| {
        !grant.scope().environment_ids().is_empty()
            && grant.authorizes_operation(AgentOperation::Inspect, || LeaseClock::now().ok())
    })
}

pub(crate) async fn list(
    actor: AuthenticatedActor,
    State(state): State<Arc<ServiceState>>,
    Query(query): Query<EnvironmentListQuery>,
) -> Result<Json<EnvironmentListResponse>, AppError> {
    let delegated = actor.agent_grant();
    if delegated.is_some() {
        check_scope(&actor)?;
    } else {
        actor.require_operator()?;
    }

    query
        .validate()
        .map_err(|message| AppError::new(StatusCode::BAD_REQUEST, message.into()))?;
    let max_scan = ENVIRONMENT_HEAD_PAGE_MAX - 1;
    let limit = query.limit.unwrap_or(24);
    let search = query.search.as_deref().unwrap_or_default();
    let search = search.trim().to_lowercase();
    let scan = if search.is_empty() { limit } else { max_scan };
    let mut counts = delegated.is_none().then(|| instance_counts(&state));
    let ids = match delegated {
        // A grant's bounded explicit list is the paging source. It cannot observe
        // another head's UUID via a page boundary or a global count.
        Some(grant) => {
            let mut ids: Vec<_> = grant
                .scope()
                .environment_ids()
                .iter()
                .filter(|id| query.after.as_ref().is_none_or(|after| *id > after))
                .cloned()
                .collect();
            ids.sort_unstable();
            ids.truncate(usize::from(scan) + 1);
            ids.into_iter().map(|id| (id, None)).collect::<Vec<_>>()
        }
        None => state
            .host_ledger
            .environment_references(EnvironmentReferenceQuery::CurrentEnvironments {
                after: query.after,
                limit: scan + 1,
            })
            .await
            .map_err(unavailable)?
            .into_iter()
            .map(|record| {
                let EnvironmentReferenceRecord::CurrentEnvironment { revision } = record else {
                    return Err(unavailable("unexpected environment head record"));
                };
                Ok((revision.environment_id, Some(revision.revision_id)))
            })
            .collect::<Result<Vec<_>, AppError>>()?,
    };

    let mut remaining = ids.into_iter();
    let mut environments = Vec::new();
    let mut last = None;
    for _ in 0..scan {
        let Some((id, known)) = remaining.next() else { break };
        if delegated.is_some() {
            check_scope(&actor)?;
        }
        last = Some(id.clone());
        let (revision_id, summary) = match state.environments.load_current(&id).await {
            Ok(current) => {
                let spec = current.document.specification;
                (
                    current.revision.revision_id,
                    Some(EnvironmentListSummary {
                        name: spec.name,
                        description: spec.description,
                        image: spec.image,
                        timestamp_unix_ms: current.revision.timestamp_unix_ms,
                    }),
                )
            }
            Err(error) => {
                let Some(revision) = known else { continue };
                tracing::debug!(environment_id = %id, %error, "environment inventory item unavailable");
                (revision, None)
            }
        };
        if !matches_search(&id, summary.as_ref(), &search) {
            continue;
        }
        let archived = state.environments.is_archived(&id).await.map_err(unavailable)?;
        let instance_count = counts.as_mut().map(|counts| counts.remove(&id).unwrap_or_default());
        environments.push(EnvironmentListItem {
            archived,
            environment: EnvironmentRevisionReference {
                environment_id: id,
                revision_id,
            },
            summary,
            instance_count,
        });
        if environments.len() == usize::from(limit) {
            break;
        }
    }
    if delegated.is_some() {
        check_scope(&actor)?;
    }
    Ok(Json(EnvironmentListResponse {
        environments,
        next_cursor: if remaining.len() > 0 { last } else { None },
    }))
}

fn check_scope(actor: &AuthenticatedActor) -> Result<(), AppError> {
    permits_list(actor)
        .then_some(())
        .ok_or_else(crate::caller_auth::session::denied)
}

fn matches_search(id: &str, summary: Option<&EnvironmentListSummary>, search: &str) -> bool {
    id.contains(search)
        || summary.is_some_and(|summary| {
            summary.name.to_lowercase().contains(search)
                || summary.description.to_lowercase().contains(search)
                || summary
                    .image
                    .as_ref()
                    .is_some_and(|image| image.reference.to_lowercase().contains(search))
        })
}

fn instance_counts(state: &ServiceState) -> HashMap<String, u32> {
    let registry = state.persistent_registry.lock().unwrap();
    let instances = state.instances.lock().unwrap();
    let live = instances.values().filter_map(|instance| {
        instance
            .environment
            .as_ref()
            .map(|binding| &binding.desired.environment_id)
    });
    let inactive = registry
        .list()
        .filter(|entry| !instances.contains_key(&crate::vm_lifecycle::persistent_entry_vm_id(entry)))
        .filter_map(|entry| {
            entry
                .environment
                .as_ref()
                .map(|environment| &environment.environment_id)
        });
    let mut counts = HashMap::<String, u32>::new();
    for id in live.chain(inactive) {
        let count = counts.entry(id.clone()).or_default();
        *count = count.saturating_add(1);
    }
    counts
}
