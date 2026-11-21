//! Descriptor-driven plugin inspection of the saved blueprint or new defaults.
use super::*;
use axum::extract::Query;
use capsem_api::environments::{
    EnvironmentPluginDescriptor, EnvironmentPluginDirectionState, EnvironmentPluginEntry, EnvironmentPluginFieldValue,
    EnvironmentPluginListQuery, EnvironmentPluginListResponse, EnvironmentRevisionReference,
};
use capsem_config::plugins::{plugin_descriptors, EnvironmentPluginSource, EnvironmentPlugins, PluginDirection};
use capsem_core::{environment::CompiledEnvironmentPlugins, net::policy_config::load_corp_policy_files};
use std::collections::BTreeMap;

pub(crate) async fn defaults(
    actor: AuthenticatedActor,
    State(state): State<Arc<ServiceState>>,
) -> Result<Json<EnvironmentPluginListResponse>, AppError> {
    actor.require_operator()?;
    let plugins = read(&state, EnvironmentPlugins::default()).await?;
    Ok(Json(EnvironmentPluginListResponse {
        revision: None,
        plugins,
    }))
}

pub(crate) async fn list(
    actor: AuthenticatedActor,
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    Query(query): Query<EnvironmentPluginListQuery>,
) -> Result<Json<EnvironmentPluginListResponse>, AppError> {
    let Json(current) = super::info(actor.clone(), State(state.clone()), Path(id.clone())).await?;
    let revision_id = current.document.identity.revision_id;
    if let Some(expected) = query.revision_id {
        EnvironmentRevisionReference {
            environment_id: id.clone(),
            revision_id: expected.clone(),
        }
        .validate()
        .map_err(|error| AppError::new(StatusCode::BAD_REQUEST, error.into()))?;
        if expected != revision_id {
            return Err(AppError::new(
                StatusCode::CONFLICT,
                "saved environment revision changed".into(),
            ));
        }
    }
    let resolved = read(&state, current.document.specification.plugins).await;
    authorize(&actor, &id)?;
    Ok(Json(EnvironmentPluginListResponse {
        revision: Some(EnvironmentRevisionReference {
            environment_id: id,
            revision_id,
        }),
        plugins: resolved?,
    }))
}

async fn read(state: &Arc<ServiceState>, plugins: EnvironmentPlugins) -> Result<Vec<EnvironmentPluginEntry>, AppError> {
    state
        .off_worker(move |_| {
            let corporate = load_corp_policy_files()?;
            let compiled = CompiledEnvironmentPlugins::with_corporate(&plugins, &corporate)?;
            // Project the typed configuration onto descriptor paths for the JSON wire contract.
            // Only declared plugin fields can appear in the response.
            let configured = serde_json::to_value(&plugins).map_err(|error| error.to_string())?;
            let overrides = serde_json::to_value(&corporate.environment_plugins).map_err(|error| error.to_string())?;
            plugin_descriptors()
                .iter()
                .map(|descriptor| {
                    let descriptor = EnvironmentPluginDescriptor::try_from(descriptor)?;
                    let resolved = compiled.configuration(descriptor.id, PluginDirection::Inbound);
                    let configured = fields(&descriptor, configured.get(descriptor.id.as_str()))?;
                    let effective = match resolved.source {
                        EnvironmentPluginSource::Builtin => descriptor
                            .fields
                            .iter()
                            .map(|field| (field.id.clone(), field.default_value.clone()))
                            .collect(),
                        EnvironmentPluginSource::Environment => configured.clone(),
                        EnvironmentPluginSource::Corporate => {
                            fields(&descriptor, overrides.get(descriptor.id.as_str()))?
                        }
                    };
                    let directions = descriptor
                        .directions
                        .iter()
                        .map(|&direction| {
                            let configuration = compiled.configuration(descriptor.id, direction);
                            EnvironmentPluginDirectionState {
                                direction,
                                mode: configuration.mode,
                                detection_level: configuration.active_detection_level(),
                            }
                        })
                        .collect();
                    Ok(EnvironmentPluginEntry {
                        descriptor,
                        source: resolved.source,
                        locked: resolved.locked(),
                        configured,
                        effective,
                        directions,
                    })
                })
                .collect::<Result<Vec<_>, String>>()
        })
        .await?
        .map_err(unavailable)
}

fn fields(
    descriptor: &EnvironmentPluginDescriptor,
    entry: Option<&serde_json::Value>,
) -> Result<BTreeMap<String, EnvironmentPluginFieldValue>, String> {
    let Some(entry) = entry else {
        return Ok(BTreeMap::new());
    };
    descriptor
        .fields
        .iter()
        .map(|field| {
            let id = &field.id;
            let value = id
                .split('.')
                .try_fold(entry, |node, part| node.get(part))
                .ok_or_else(|| {
                    format!(
                        "plugin {} descriptor field {id} has no typed value",
                        descriptor.id.as_str()
                    )
                })?;
            serde_json::from_value(value.clone())
                .map(|value| (id.clone(), value))
                .map_err(|error| error.to_string())
        })
        .collect()
}
