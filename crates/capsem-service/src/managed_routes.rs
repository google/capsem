//! Public managed ownership routes use the existing durable lifecycle owner.
use super::*;
use capsem_core::managed_sessions::{
    controller::{Controller, EffectBounds, Failure},
    Capability, LeasePolicy, Registry, Snapshot, State as OwnershipState,
};
use capsem_core::net::policy_config::ManagedSessionPolicy;
use std::time::{Duration, UNIX_EPOCH};

pub(crate) struct Runtime {
    controller: Controller,
    grants: Arc<capsem_credentials::GrantAuthority>,
    policy: ManagedSessionPolicy,
}

impl Runtime {
    pub(crate) fn open(run_dir: &StdPath, policy: ManagedSessionPolicy) -> Result<Self> {
        policy.validate().map_err(anyhow::Error::msg)?;
        let lifetime = Duration::from_secs(u64::from(policy.lease_seconds));
        Ok(Self {
            controller: Controller::new(
                Registry::open(&run_dir.join("managed-ownership"))?,
                EffectBounds::new(lifetime, lifetime)?,
            ),
            // This authority does not issue connections or grants implicitly.
            // All retirement uses the same generation-specific broker owner.
            grants: Arc::new(capsem_credentials::GrantAuthority::new(
                capsem_credentials::GrantLimits {
                    max_grants: 1024,
                    max_lifetime: lifetime,
                },
            )?),
            policy,
        })
    }

    pub(crate) fn start_maintenance(state: Arc<ServiceState>) {
        tokio::spawn(async move {
            let runtime = &state.managed;
            // Recovery is cleanup-only. Missing original child authority must
            // remain pending rather than manufacturing an absence barrier.
            match runtime.controller.recovery_inventory().await {
                Ok(inventory) => {
                    for snapshot in inventory {
                        if snapshot.state() == OwnershipState::Closed {
                            continue;
                        }
                        if let Ok(effects) = managed_sessions::ManagedLifecycle::for_recovery_with_broker(
                            Arc::clone(&state),
                            &snapshot,
                            Arc::clone(&runtime.grants),
                        ) {
                            let controller = runtime.controller.clone();
                            tokio::spawn(async move {
                                if let Err(error) = controller.recover(snapshot.request(), Arc::new(effects)).await {
                                    warn!(%error, "managed startup cleanup remains pending");
                                }
                            });
                        }
                    }
                }
                Err(error) => warn!(%error, "managed startup inventory unavailable"),
            }
            let mut interval =
                tokio::time::interval(Duration::from_secs(u64::from(runtime.policy.renew_after_seconds)));
            loop {
                interval.tick().await;
                if let Err(error) = runtime
                    .controller
                    .prune_closed(Duration::from_secs(u64::from(runtime.policy.tombstone_seconds)))
                    .await
                {
                    warn!(%error, "managed tombstone retention remains pending");
                }
            }
        });
    }
}

pub(crate) struct ManagedError(StatusCode, api::ManagedSessionFailure);

impl ManagedError {
    pub(crate) fn invalid() -> Self {
        Self(
            StatusCode::BAD_REQUEST,
            api::ManagedSessionFailure {
                code: api::ManagedSessionErrorCode::InvalidRequest,
                error: "invalid managed session request".into(),
            },
        )
    }
}

impl From<Failure> for ManagedError {
    fn from(error: Failure) -> Self {
        use api::ManagedSessionErrorCode as Code;
        let (status, code) = match error {
            Failure::Forbidden => (StatusCode::FORBIDDEN, Code::Forbidden),
            Failure::NotFound => (StatusCode::NOT_FOUND, Code::NotFound),
            Failure::DuplicateRequest => (StatusCode::CONFLICT, Code::DuplicateRequest),
            Failure::CreationFailed => (StatusCode::INTERNAL_SERVER_ERROR, Code::CreationFailed),
            Failure::CleanupFailed => (StatusCode::INTERNAL_SERVER_ERROR, Code::CleanupFailed),
            Failure::RecoveryPending => (StatusCode::INTERNAL_SERVER_ERROR, Code::RecoveryPending),
            Failure::StoreUnavailable => (StatusCode::INTERNAL_SERVER_ERROR, Code::StoreUnavailable),
        };
        Self(
            status,
            api::ManagedSessionFailure {
                code,
                error: error.to_string(),
            },
        )
    }
}

impl IntoResponse for ManagedError {
    fn into_response(self) -> axum::response::Response {
        // Only the typed, sanitized diagnostic is available here.
        warn!(status = self.0.as_u16(), code = ?self.1.code, "managed request failed");
        (self.0, Json(self.1)).into_response()
    }
}

fn request_id(value: &str) -> Result<uuid::Uuid, ManagedError> {
    let id = uuid::Uuid::parse_str(value).map_err(|_| ManagedError::invalid())?;
    if id.get_version_num() != 4 || id.get_variant() != uuid::Variant::RFC4122 || id.to_string() != value {
        return Err(ManagedError::invalid());
    }
    Ok(id)
}

fn capability(token: &api::ManagedOwnershipToken) -> Result<Capability, ManagedError> {
    let mut bytes = [0; 32];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte =
            u8::from_str_radix(&token.as_str()[index * 2..index * 2 + 2], 16).map_err(|_| ManagedError::invalid())?;
    }
    Ok(Capability::from_bytes(bytes))
}

fn ownership(snapshot: &Snapshot, policy: ManagedSessionPolicy) -> Result<api::ManagedSessionOwnership, ManagedError> {
    let expiry = UNIX_EPOCH
        .checked_add(Duration::from_millis(snapshot.expires_wall_ms().unwrap_or(0)))
        .ok_or(Failure::StoreUnavailable)?;
    Ok(api::ManagedSessionOwnership {
        request_id: snapshot.request().to_string(),
        generation: snapshot.generation().to_string(),
        expires_at: humantime::format_rfc3339_millis(expiry).to_string(),
        lease_seconds: policy.lease_seconds,
        renew_after_seconds: policy.renew_after_seconds,
    })
}

fn session(state: &ServiceState, snapshot: &Snapshot) -> Result<Option<ProvisionResponse>, ManagedError> {
    let Some(binding) = snapshot.vm() else { return Ok(None) };
    let instances = state.instances.lock().unwrap();
    let Some(instance) = instances.get(binding.id()) else {
        return Ok(None);
    };
    if instance.generation != binding.generation() || instance.persistent {
        return Err(Failure::RecoveryPending.into());
    }
    let mut response = ProvisionResponse::new(
        instance.id.clone(),
        instance.name.clone(),
        VmLifecycleState::Running,
        false,
        false,
        VmLifecycleState::Running.available_actions(false),
    );
    drop(instances);
    response.ownership = Some(ownership(snapshot, state.managed.policy)?);
    Ok(Some(response))
}

pub(crate) async fn create(
    state: Arc<ServiceState>,
    mut payload: ProvisionRequest,
) -> Result<Json<ProvisionResponse>, ManagedError> {
    if payload.name.is_some() || payload.persistent {
        return Err(ManagedError::invalid());
    }
    let managed = payload.managed.take().ok_or_else(ManagedError::invalid)?;
    let request = request_id(&managed.request_id)?;
    let token = capability(&managed.ownership_token)?;
    let effects = managed_sessions::ManagedLifecycle::new_with_broker(
        Arc::clone(&state),
        payload,
        Arc::clone(&state.managed.grants),
    )
    .map_err(|_| ManagedError::invalid())?;
    let policy = LeasePolicy::new(Duration::from_secs(u64::from(state.managed.policy.lease_seconds)))
        .map_err(|_| ManagedError::invalid())?;
    let snapshot = state
        .managed
        .controller
        .create(request, token, policy, Arc::new(effects))
        .await?;
    if snapshot.state() != OwnershipState::Active {
        return Err(Failure::CreationFailed.into());
    }
    Ok(Json(session(&state, &snapshot)?.ok_or(Failure::RecoveryPending)?))
}

pub(crate) async fn claim(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    payload: Result<Json<api::ManagedSessionClaimRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<api::ManagedSessionClaimResponse>, ManagedError> {
    let request = request_id(&id)?;
    let Json(payload) = payload.map_err(|_| ManagedError::invalid())?;
    let snapshot = state
        .managed
        .controller
        .claim(request, capability(&payload.ownership_token)?)
        .await?;
    let status = match snapshot.state() {
        OwnershipState::Reserved | OwnershipState::Creating => api::ManagedSessionState::Pending,
        OwnershipState::Active => api::ManagedSessionState::Active,
        OwnershipState::Closing => api::ManagedSessionState::Closing,
        OwnershipState::Closed => api::ManagedSessionState::Closed,
        OwnershipState::Unknown => api::ManagedSessionState::Failed,
    };
    let error = if snapshot.state() == OwnershipState::Unknown {
        Some(ManagedError::from(Failure::CreationFailed).1)
    } else {
        None
    };
    Ok(Json(api::ManagedSessionClaimResponse {
        ownership: ownership(&snapshot, state.managed.policy)?,
        state: status,
        session: session(&state, &snapshot)?,
        error,
    }))
}

pub(crate) async fn close(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    payload: Result<Json<api::ManagedSessionCloseRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<api::ManagedSessionCloseResponse>, ManagedError> {
    let request = request_id(&id)?;
    let Json(payload) = payload.map_err(|_| ManagedError::invalid())?;
    let snapshot = state
        .managed
        .controller
        .close(request, capability(&payload.ownership_token)?)
        .await?;
    if snapshot.state() != OwnershipState::Closed {
        return Err(Failure::CleanupFailed.into());
    }
    // The outcome reports why the caller closed. It grants no preservation or
    // persistence authority. Crash evidence is owned by the existing reaper.
    let binding = snapshot.spawn_intent().cloned();
    let evidence = binding.clone();
    let evidence_retained = state
        .off_worker(move |state| -> Result<bool> {
            use capsem_foundation::unix::contained::{ContainedDir, EntryKind};
            let Some(binding) = evidence else { return Ok(false) };
            let root = match ContainedDir::open_root(&state.run_dir.join("sessions")) {
                Ok(root) => root,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
                Err(error) => return Err(error.into()),
            };
            let prefix = format!("{}-failed-", binding.id());
            for entry in root.entries()? {
                if entry.kind != EntryKind::Directory || !entry.name.to_string_lossy().starts_with(&prefix) {
                    continue;
                }
                let directory = root.descend(&entry.name)?;
                if capsem_core::session::read_spawn_identity(&directory)?.as_ref() == Some(&binding) {
                    return Ok(true);
                }
            }
            Ok(false)
        })
        .await
        .map_err(|_| Failure::CleanupFailed)?
        .map_err(|_| Failure::CleanupFailed)?;
    Ok(Json(api::ManagedSessionCloseResponse {
        request_id: snapshot.request().to_string(),
        generation: snapshot.generation().to_string(),
        state: api::ManagedSessionClosedState::Closed,
        session_id: binding.map(|binding| binding.id().to_owned()),
        evidence_retained,
    }))
}

#[cfg(test)]
mod tests;
