//! Inspect the rules compiled for launch without publishing to a running VM.
use super::*;
use axum::extract::Query;
use capsem_api::environments::{
    EnvironmentEffectiveAccessRule, EnvironmentEffectiveDetectionRule, EnvironmentRuleListResponse,
    EnvironmentRuleSource,
};
use capsem_core::{environment::EnvironmentPolicyInputs, net::policy_config::load_corp_policy_files};

pub(crate) async fn list(
    actor: AuthenticatedActor,
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    Query(query): Query<EnvironmentPolicyListQuery>,
) -> Result<Json<EnvironmentRuleListResponse>, AppError> {
    let (revision, specification) = policy_document(&actor, &state, id, query).await?;
    let compiled = read(&state, &specification).await;
    authorize(&actor, &revision.environment_id)?;
    let (access, detection) = compiled?;
    Ok(Json(EnvironmentRuleListResponse {
        revision,
        access,
        detection,
    }))
}

async fn read(
    state: &Arc<ServiceState>,
    specification: &EnvironmentSpecification,
) -> Result<
    (
        Vec<EnvironmentEffectiveAccessRule>,
        Vec<EnvironmentEffectiveDetectionRule>,
    ),
    AppError,
> {
    let corporate = state
        .off_worker(|_| load_corp_policy_files())
        .await?
        .map_err(unavailable)?;
    let inputs = EnvironmentPolicyInputs::new(specification, specification, &corporate);
    // Explicit rule inspection refreshes admission when the revision selects a catalog.
    // As on launch, the current tag admits a selection but never replaces its pinned definition.
    let policy = launch_policy(state, &inputs)
        .await
        .map_err(|error| mutation_error(EnvironmentMutationError::Resolution(error)))?;
    state
        .off_worker(move |_| {
            if load_corp_policy_files().map_err(unavailable)? != corporate {
                return Err(AppError::new(
                    StatusCode::CONFLICT,
                    "corporate policy changed during rule inspection".into(),
                ));
            }
            let rules = policy.compile_security_rule_set().map_err(unavailable)?;
            let access = rules
                .rules()
                .iter()
                .map(|rule| {
                    use EnvironmentRuleSource::*;
                    let source = if rule.corp_locked {
                        Corporate
                    } else {
                        match rule.namespace.as_str() {
                            "environment.access" => Environment,
                            "environment.access.catalog" => Catalog,
                            "environment.mcps" => EnvironmentMcp,
                            _ => Builtin,
                        }
                    };
                    EnvironmentEffectiveAccessRule {
                        id: rule.rule_id.clone(),
                        name: rule.name.clone(),
                        condition: rule.condition.clone(),
                        action: rule.action,
                        priority: rule.priority,
                        enabled: rule.enabled,
                        source,
                        locked: rule.corp_locked,
                        detection_level: rule.detection_level,
                        editable_key: (source == Environment).then(|| rule.rule_key.clone()),
                    }
                })
                .collect();
            let detection = rules
                .observations()
                .iter()
                .map(|rule| {
                    let key = rule
                        .rule_id
                        .strip_prefix("environment.detection.")
                        .filter(|key| !key.starts_with("catalog."));
                    EnvironmentEffectiveDetectionRule {
                        id: rule.rule_id.clone(),
                        name: rule.name.clone(),
                        condition: rule.condition.clone(),
                        level: rule.level,
                        enabled: rule.enabled,
                        source: if key.is_some() {
                            EnvironmentRuleSource::Environment
                        } else {
                            EnvironmentRuleSource::Catalog
                        },
                        editable_key: key.map(String::from),
                    }
                })
                .collect();
            Ok((access, detection))
        })
        .await?
}
