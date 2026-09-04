use super::*;

fn cache_error(name: &str, error: impl std::fmt::Display) -> AppError {
    AppError(
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("{name} cache lock poisoned: {error}"),
    )
}

fn rule_backed_mutation(target_kind: &str) -> bool {
    matches!(target_kind, "mcp_default" | "mcp_tool" | "security_rule")
}

fn update_profile_summary(
    state: &ServiceState,
    profile: &Profile,
    rules: Option<&[api::EnforcementRuleInfo]>,
    target_kind: &str,
) -> Result<(), AppError> {
    if target_kind != "mcp_server" && rules.is_none() {
        return Ok(());
    }
    let profile_id = profile.config().id.as_str();
    let mut summaries = state
        .profile_summary_cache
        .lock()
        .map_err(|error| cache_error("profile summary", error))?;
    let summary = summaries
        .iter_mut()
        .find(|summary| summary.id == profile_id)
        .ok_or_else(|| AppError(StatusCode::NOT_FOUND, format!("profile not found: {profile_id}")))?;
    if target_kind == "mcp_server" {
        summary.mcp_server_count = profile.config().mcp.as_ref().map_or(0, |mcp| {
            mcp.servers.len() + usize::from(mcp.server_enabled.get("local").copied().unwrap_or(false))
        });
    }
    if let Some(rules) = rules {
        summary.rule_count = rules.len();
        summary.default_rule_count = rules.iter().filter(|rule| rule.default_rule).count();
    }
    drop(summaries);
    Ok(())
}

fn refresh_rule_caches(state: &ServiceState, profile: &Profile) -> Result<Vec<api::EnforcementRuleInfo>, AppError> {
    let (_, corp) = capsem_core::net::policy_config::load_settings_and_corp_files();
    let rules = list_enforcement_rules_for_profile_config(profile, &corp)?;
    let permission = profile
        .mcp_default_permission()
        .map(|permission| api::McpDefaultPermissionResponse {
            action: permission.action,
            source: permission.source,
            rule_id: permission.rule_id,
        });
    state
        .profile_rule_cache
        .lock()
        .map_err(|error| cache_error("profile rule", error))?
        .insert(profile.config().id.clone(), rules.clone());
    state
        .profile_mcp_default_cache
        .lock()
        .map_err(|error| cache_error("profile MCP default", error))?
        .insert(profile.config().id.clone(), permission);
    Ok(rules)
}

/// Publish one already-validated profile mutation into the route caches.
///
/// Mutation handlers already own the updated `Profile`. Reusing that typed
/// value avoids reloading every profile, rehashing all assets, and rebuilding
/// unrelated route projections after each write. Asset/status data is marked
/// stale and rebuilt lazily by its owning read route.
pub(super) fn refresh_after_profile_mutation(
    state: &ServiceState,
    profile: &Profile,
    event: &capsem_logger::ProfileMutationEvent,
) -> Result<(), AppError> {
    if profile.config().id != event.profile_id {
        return Err(AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!(
                "profile mutation cache identity mismatch: profile={} event={}",
                profile.config().id,
                event.profile_id
            ),
        ));
    }

    state
        .profile_cache
        .lock()
        .map_err(|error| cache_error("profile", error))?
        .insert(event.profile_id.clone(), profile.clone());

    let rules = if rule_backed_mutation(&event.target_kind) {
        Some(refresh_rule_caches(state, profile)?)
    } else {
        None
    };
    update_profile_summary(state, profile, rules.as_deref(), &event.target_kind)?;

    if event.target_kind == "plugin" {
        state
            .profile_plugin_policy_cache
            .lock()
            .map_err(|error| cache_error("profile plugin", error))?
            .insert(event.profile_id.clone(), profile.config().plugins.clone());
    }

    *state
        .profile_status_cache
        .lock()
        .map_err(|error| cache_error("profile status", error))? = None;
    state.persistent_resume_state_cache.lock().unwrap().clear();
    if rules.is_some() {
        state.profile_rule_response_cache.lock().unwrap().clear();
    }
    if event.target_kind == "plugin" {
        state.profile_plugin_response_cache.lock().unwrap().clear();
    }
    if rules.is_some() || event.target_kind == "plugin" {
        state.evaluate_response_cache.lock().unwrap().clear();
        *state.evaluate_last_response_cache.lock().unwrap() = None;
    }
    Ok(())
}
