//! Approved authenticated connection/grant routes, with no request-owned tasks.
use super::*;
use capsem_core::managed_sessions::{controller::CredentialOwner, VmBinding};
use capsem_credentials::broker::{BrokerLimits, BrokerProfile, BrokerStorage, ConnectionBroker, SessionBinding};
use std::time::SystemTime;
pub(crate) mod lifecycle;
pub(crate) mod policy;

pub(crate) struct Runtime {
    path: PathBuf,
    authority: Arc<capsem_credentials::GrantAuthority>,
    broker: tokio::sync::OnceCell<Result<ConnectionBroker, api::CredentialErrorCode>>,
    applied: std::sync::Mutex<std::collections::BTreeMap<(String, uuid::Uuid), policy::Applied>>,
}
impl Runtime {
    pub(crate) fn new(path: PathBuf, authority: Arc<capsem_credentials::GrantAuthority>) -> Self {
        Self {
            path,
            authority,
            broker: tokio::sync::OnceCell::new(),
            applied: Default::default(),
        }
    }
    async fn broker(&self) -> Result<&ConnectionBroker, CredentialError> {
        self.broker
            .get_or_init(|| async {
                let limits = BrokerLimits::bundled()?;
                let profile = BrokerProfile::unconfigured(api::ScopeProfileSummary {
                    id: "agy-native".into(),
                    enrollment_kind: api::EnrollmentKind::NativeConsumerSignIn,
                    label: "Antigravity".into(),
                    application_label: "Google Antigravity".into(),
                    requested_scopes: vec![],
                    capabilities: vec![api::CapabilitySummary {
                        id: "agy".into(),
                        label: "Antigravity".into(),
                        consumer: "agy".into(),
                        required_scopes: vec![],
                        supported: false,
                    }],
                    enabled: false,
                })?;
                ConnectionBroker::open(
                    BrokerStorage::File {
                        path: self.path.clone(),
                        max_bytes: limits.max_metadata_bytes,
                    },
                    vec![profile],
                    Arc::clone(&self.authority),
                    limits,
                )
                .await
            })
            .await
            .as_ref()
            .map_err(|code| CredentialError(*code, false))
    }
    pub(crate) async fn start(state: Arc<ServiceState>) {
        let Ok(limits) = BrokerLimits::bundled() else {
            warn!("connection maintenance policy unavailable");
            return;
        };
        if let Err(error) = state.connections.broker().await {
            warn!(code=?error.0,"connection owner unavailable");
            return;
        }
        tokio::spawn(async move {
            loop {
                if let Ok(broker) = state.connections.broker().await {
                    if let Err(code) = broker.prune().await {
                        warn!(?code, "connection retention remains pending");
                    }
                }
                tokio::time::sleep(limits.maintenance_interval).await;
            }
        });
    }
}

pub(crate) struct CredentialError(api::CredentialErrorCode, bool);
impl From<api::CredentialErrorCode> for CredentialError {
    fn from(code: api::CredentialErrorCode) -> Self {
        Self(code, false)
    }
}
impl IntoResponse for CredentialError {
    fn into_response(self) -> axum::response::Response {
        use api::CredentialErrorCode as Code;
        let status = if self.1 {
            StatusCode::BAD_REQUEST
        } else {
            match self.0 {
                Code::PolicyDenied | Code::ScopeDenied => StatusCode::FORBIDDEN,
                Code::UnknownScopeProfile
                | Code::ConnectionUnavailable
                | Code::GrantUnavailable
                | Code::TransactionExpired => StatusCode::NOT_FOUND,
                Code::RequestConflict
                | Code::OwnerChanged
                | Code::ReauthRequired
                | Code::TransactionCancelled
                | Code::RevocationPending => StatusCode::CONFLICT,
                Code::InvalidCallback | Code::UnsupportedCapability => StatusCode::BAD_REQUEST,
                Code::ProviderRejected => StatusCode::BAD_GATEWAY,
                Code::ProviderTimeout => StatusCode::GATEWAY_TIMEOUT,
                _ => StatusCode::SERVICE_UNAVAILABLE,
            }
        };
        warn!(code=?self.0,status=status.as_u16(),"credential request refused");
        let code: String =
            serde_json::from_str(&serde_json::to_string(&self.0).expect("wire enum")).expect("wire enum string");
        (
            status,
            Json(api::ErrorResponse {
                error: "credential request refused".into(),
                code: Some(code),
            }),
        )
            .into_response()
    }
}
fn body<T>(payload: Result<Json<T>, axum::extract::rejection::JsonRejection>) -> Result<T, CredentialError> {
    payload
        .map(|Json(value)| value)
        .map_err(|_| CredentialError(api::CredentialErrorCode::RequestConflict, true))
}

pub(crate) async fn list(
    State(state): State<Arc<ServiceState>>,
) -> Result<Json<api::ConnectionListResponse>, CredentialError> {
    Ok(Json(state.connections.broker().await?.list().await?))
}
pub(crate) async fn authorize(
    State(state): State<Arc<ServiceState>>,
    payload: Result<Json<api::AuthorizeGoogleRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<(StatusCode, Json<api::AuthorizeGoogleResponse>), CredentialError> {
    let request = body(payload)?;
    let _policy = state.policy_mutation.begin().await;
    enabled(&state).await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(state.connections.broker().await?.authorize(request).await?),
    ))
}
pub(crate) async fn authorization(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<Json<api::AuthorizationTransaction>, CredentialError> {
    Ok(Json(state.connections.broker().await?.authorization(&id).await?))
}
pub(crate) async fn cancel(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<Json<api::AuthorizationTransaction>, CredentialError> {
    Ok(Json(state.connections.broker().await?.cancel_authorization(&id).await?))
}
pub(crate) async fn get_connection(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<Json<api::ConnectionSummary>, CredentialError> {
    Ok(Json(state.connections.broker().await?.connection(&id).await?))
}
pub(crate) async fn disconnect(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<api::DisconnectResponse>), CredentialError> {
    Ok((
        StatusCode::ACCEPTED,
        Json(state.connections.broker().await?.disconnect(&id).await?),
    ))
}

struct Context {
    binding: SessionBinding,
    runtime: VmBinding,
    managed: Option<CredentialOwner>,
    policy_path: PathBuf,
    uds_path: PathBuf,
}
impl Context {
    fn current(&self, state: &ServiceState) -> bool {
        if self.managed.as_ref().is_some_and(|owner| !owner.is_current()) {
            return false;
        }
        let instances = state.instances.lock().unwrap();
        let matched = instances
            .get(self.runtime.id())
            .is_some_and(|instance| instance.generation == self.runtime.generation());
        drop(instances);
        matched
            && state
                .retirements
                .is_live(self.runtime.id(), self.runtime.generation())
                .unwrap_or(false)
    }
}
async fn context(state: &ServiceState, id: &str) -> Result<Context, CredentialError> {
    let session =
        uuid::Uuid::parse_str(id).map_err(|_| CredentialError(api::CredentialErrorCode::SessionUnavailable, false))?;
    if session.is_nil() || session.to_string() != id {
        return Err(api::CredentialErrorCode::SessionUnavailable.into());
    }
    let (generation, policy_path, uds_path) = state
        .instances
        .lock()
        .unwrap()
        .get(id)
        .map(|instance| {
            (
                instance.generation,
                instance.session_dir.join(ACTIVE_POLICY_DIR).join(ACTIVE_POLICY_FILE),
                instance.uds_path.clone(),
            )
        })
        .ok_or(api::CredentialErrorCode::SessionUnavailable)?;
    let runtime = VmBinding::new(id.to_owned(), generation)
        .map_err(|_| CredentialError(api::CredentialErrorCode::SessionUnavailable, false))?;
    let managed = state
        .managed
        .credential_owner(&runtime)
        .await
        .map_err(|_| CredentialError(api::CredentialErrorCode::SessionUnavailable, false))?;
    let (owner, lifetime) = match &managed {
        Some(owner) => (
            owner.generation(),
            owner.remaining().ok_or(api::CredentialErrorCode::SessionUnavailable)?,
        ),
        // Ordinary identity is the immutable service-issued canonical session.
        // Display names and attaching clients never create managed authority.
        None => (session, BrokerLimits::bundled()?.grant_lifetime),
    };
    let grant = capsem_credentials::GrantSession::new(*session.as_bytes(), *owner.as_bytes(), *generation.as_bytes())
        .map_err(|_| CredentialError(api::CredentialErrorCode::SessionUnavailable, false))?;
    let context = Context {
        binding: SessionBinding::new(grant, SystemTime::now() + lifetime)?,
        runtime,
        managed,
        policy_path,
        uds_path,
    };
    if !context.current(state) {
        return Err(api::CredentialErrorCode::SessionUnavailable.into());
    }
    Ok(context)
}
async fn enabled(state: &Arc<ServiceState>) -> Result<(), CredentialError> {
    let permitted = state
        .off_worker(|_| {
            let (user, corp) = capsem_core::net::policy_config::load_policy_files()?;
            let plugins = capsem_core::net::policy_config::merge_plugin_policy(&user, &corp);
            Ok::<_, String>(
                plugins
                    .get("credential_broker")
                    .is_some_and(|plugin| plugin.mode == SecurityPluginMode::Rewrite),
            )
        })
        .await
        .map_err(|_| CredentialError(api::CredentialErrorCode::StorageUnavailable, false))?
        .map_err(|_| CredentialError(api::CredentialErrorCode::PolicyDenied, false))?;
    if !permitted {
        return Err(api::CredentialErrorCode::PolicyDenied.into());
    }
    Ok(())
}
pub(crate) async fn grants(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<Json<api::SessionGrantListResponse>, CredentialError> {
    let _policy = state.policy_mutation.begin().await;
    enabled(&state).await?;
    let context = context(&state, &id).await?;
    let broker = state.connections.broker().await?;
    let admission = policy::admission(&state, &context, broker, false).await?;
    Ok(Json(
        state
            .connections
            .broker()
            .await?
            .grants(context.binding, |_, capability| {
                context.current(&state) && admission.admits(capability)
            })
            .await?,
    ))
}
pub(crate) async fn create_grant(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    payload: Result<Json<api::CreateSessionGrantRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<(StatusCode, Json<api::SessionGrantSummary>), CredentialError> {
    let request = body(payload)?;
    let _policy = state.policy_mutation.begin().await;
    enabled(&state).await?;
    let context = context(&state, &id).await?;
    let broker = state.connections.broker().await?;
    let admission = policy::admission(&state, &context, broker, true).await?;
    Ok((
        StatusCode::CREATED,
        Json(
            state
                .connections
                .broker()
                .await?
                .create_grant(context.binding, request, |_, capability| {
                    context.current(&state) && admission.admits(capability)
                })
                .await?,
        ),
    ))
}
pub(crate) async fn revoke_grant(
    State(state): State<Arc<ServiceState>>,
    Path((id, grant)): Path<(String, String)>,
) -> Result<Json<api::RevokeSessionGrantResponse>, CredentialError> {
    let context = context(&state, &id).await?;
    Ok(Json(
        state
            .connections
            .broker()
            .await?
            .revoke_grant(context.binding, &grant)
            .await?,
    ))
}

#[cfg(test)]
mod tests;
