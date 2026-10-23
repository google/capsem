//! Authenticated decisions address an original waiter in one boot generation.
use super::*;
use capsem_core::security_engine::approval::{ApprovalBinding, ApprovalSnapshot};

fn binding(state: &ServiceState, id: &str) -> Result<(ApprovalBinding, PathBuf), AppError> {
    let canonical = uuid::Uuid::parse_str(id).is_ok_and(|value| !value.is_nil() && value.to_string() == id);
    if !canonical {
        return Err(AppError::vm_not_found(id));
    }
    let instances = state.instances.lock().unwrap();
    let instance = instances.get(id).ok_or_else(|| AppError::vm_not_found(id))?;
    if instance.authority.grant().is_revoked() {
        return Err(AppError::vm_not_found(id));
    }
    let binding = (
        ApprovalBinding {
            session_id: id.into(),
            generation: instance.generation,
        },
        instance.session_dir.clone(),
    );
    drop(instances);
    Ok(binding)
}

async fn rows(
    state: &ServiceState,
    binding: &ApprovalBinding,
    directory: &StdPath,
    limit: u16,
    ask_id: Option<String>,
) -> Result<Vec<capsem_logger::SecurityAskView>, AppError> {
    let path = directory.join("session.db");
    let db = ledger_routes::session_db(state, &binding.session_id, "approval", &path).await?;
    ledger_routes::rows::query_route_typed_rows(
        &binding.session_id,
        "approval",
        "asks",
        &path,
        &db,
        capsem_logger::ledger_protocol::LedgerQuery::SecurityAsks { limit, ask_id },
        0,
    )
    .await
}

fn request(row: capsem_logger::SecurityAskView, active: Option<ApprovalSnapshot>) -> api::SecurityAskRequest {
    use api::SecurityAskStatus as Status;
    use capsem_logger::SecurityAskStatus as Stored;
    let created = active.as_ref().map_or(row.created_at_unix_ms, |value| value.created_ms);
    let expires = active.as_ref().map_or_else(
        || created.saturating_add((capsem_proto::APPROVAL_TIMEOUT_SECONDS * 1000) as i64),
        |value| value.expires_ms,
    );
    let status = match row.event.status {
        Stored::Approved => Status::Approved,
        Stored::Denied => match row.event.reason.as_deref() {
            Some("deadline") => Status::Expired,
            Some("request_cancelled") => Status::Cancelled,
            _ => Status::Denied,
        },
        Stored::Pending if active.is_some() => Status::Pending,
        Stored::Pending => Status::Cancelled,
    };
    let summary = active.map_or_else(
        || format!("{} request", row.event.event_type),
        |value| value.request.summary,
    );
    // Resolution reasons are service-owned codes, never caller-supplied text.
    let reason = row.event.reason.filter(|value| {
        matches!(
            value.as_str(),
            "deadline" | "request_cancelled" | "unavailable" | "human_decision"
        )
    });
    api::SecurityAskRequest {
        ask_id: row.event.ask_id,
        event_id: row.event.event_id,
        event_type: row.event.event_type,
        rule_id: row.event.rule_id,
        rule_name: row.event.rule_name,
        summary,
        status,
        created_at_unix_ms: created,
        expires_at_unix_ms: expires,
        resolved_at_unix_ms: (row.event.status != Stored::Pending).then_some(row.event.timestamp_unix_ms),
        resolution_reason: reason,
    }
}

pub(crate) async fn list(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    Query(query): Query<api::SecurityAskListQuery>,
) -> Result<Json<api::SecurityAskListResponse>, AppError> {
    let limit = query.limit.unwrap_or(100);
    if limit == 0 || limit > 2000 {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "approval limit must be 1..2000".into(),
        ));
    }
    let (binding, directory) = binding(&state, &id)?;
    let requests = rows(&state, &binding, &directory, limit, None)
        .await?
        .into_iter()
        .map(|row| {
            let active = state.approvals.broker.active(&binding, &row.event.ask_id);
            request(row, active)
        })
        .collect();
    Ok(Json(api::SecurityAskListResponse {
        vm_id: id,
        generation: binding.generation.to_string(),
        pending_count: state.approvals.broker.pending_count(&binding) as u32,
        requests,
    }))
}

pub(crate) async fn resolve(
    State(state): State<Arc<ServiceState>>,
    Path((id, ask_id)): Path<(String, String)>,
    Json(body): Json<api::SecurityAskResolveRequest>,
) -> Result<Json<api::SecurityAskRequest>, AppError> {
    body.validate()
        .map_err(|reason| AppError::new(StatusCode::BAD_REQUEST, reason.into()))?;
    if ask_id.len() != 12
        || !ask_id
            .bytes()
            .all(|value| value.is_ascii_digit() || (b'a'..=b'f').contains(&value))
    {
        return Err(AppError::new(StatusCode::BAD_REQUEST, "invalid approval id".into()));
    }
    let (binding, directory) = binding(&state, &id)?;
    if body.generation != binding.generation.to_string() {
        return Err(conflict());
    }
    let session = state
        .approvals
        .get(&binding)
        .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "approval not found".into()))?;
    let row = rows(&state, &binding, &directory, 1, Some(ask_id.clone()))
        .await?
        .pop()
        .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "approval not found".into()))?;
    if row.event.status != capsem_logger::SecurityAskStatus::Pending {
        return Err(conflict());
    }
    let snapshot = state.approvals.broker.active(&binding, &ask_id).ok_or_else(conflict)?;
    session
        .resolve(&ask_id, body.decision == api::SecurityAskDecision::Approve)
        .await
        .map_err(|error| match error {
            approval_runtime::ResolveError::Missing | approval_runtime::ResolveError::Conflict => conflict(),
            approval_runtime::ResolveError::Unavailable => {
                AppError::new(StatusCode::SERVICE_UNAVAILABLE, "approval audit unavailable".into())
            }
        })?;
    let row = rows(&state, &binding, &directory, 1, Some(ask_id))
        .await?
        .pop()
        .ok_or_else(|| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "approval receipt missing".into()))?;
    Ok(Json(request(row, Some(snapshot))))
}

fn conflict() -> AppError {
    AppError::new(
        StatusCode::CONFLICT,
        "approval is no longer pending in this generation".into(),
    )
}
