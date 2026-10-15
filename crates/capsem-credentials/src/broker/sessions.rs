//! Explicit durable approvals bound only to the service's canonical owner.
use super::*;

#[derive(Debug, Clone, Copy)]
pub struct SessionBinding {
    pub(super) session: GrantSession,
    pub(super) expires: SystemTime,
}
impl SessionBinding {
    /// Only the trusted service resolves this value; request bodies cannot
    /// select a runtime incarnation or manufacture its remaining lifetime.
    pub fn new(session: GrantSession, expires: SystemTime) -> BrokerResult<Self> {
        if expires <= SystemTime::now() {
            return Err(api::CredentialErrorCode::SessionUnavailable);
        }
        Ok(Self { session, expires })
    }
    pub(super) fn ids(&self) -> (String, String) {
        self.owner().ids()
    }
    pub fn owner(&self) -> ApprovalOwner {
        let (session, owner, _) = self.session.identities();
        ApprovalOwner {
            session,
            ownership: owner,
        }
    }
}

impl ConnectionBroker {
    /// Resume has no credential work when this canonical session has no
    /// explicit approval. This bounded inventory check performs no file I/O.
    pub async fn has_session_approval(&self, session: [u8; 16]) -> bool {
        let id = uuid::Uuid::from_bytes(session).to_string();
        let state = self.0.state.lock().await;
        let found = state.grants.values().any(|entry| {
            entry.record.summary.session_id == id && entry.record.summary.state == api::GrantState::Active
        });
        drop(state);
        found
    }
    /// Fence the exact runtime before awaiting metadata, preserving finite
    /// approval only when the lifecycle explicitly retains this session.
    pub async fn stop_runtime(
        &self,
        session: [u8; 16],
        runtime: [u8; 16],
        policy: crate::GrantStopPolicy,
    ) -> BrokerResult<()> {
        self.0
            .authority
            .detach_runtime(session, runtime, policy)
            .map_err(grant_error)?;
        let mut state = self.0.state.lock().await;
        let mut matched = false;
        for entry in state.grants.values_mut().filter(|entry| {
            entry.binding.is_some_and(|binding| {
                let (bound_session, _, bound_runtime) = binding.identities();
                bound_session == session && bound_runtime == runtime
            })
        }) {
            matched = true;
            match policy {
                crate::GrantStopPolicy::Revoke => self.revoke_entry(entry)?,
                crate::GrantStopPolicy::Preserve if entry.record.summary.runtime_bound => {
                    entry.record.summary.runtime_bound = false;
                    entry.record.summary.revision = next_revision(entry.record.summary.revision)?;
                }
                crate::GrantStopPolicy::Preserve => {}
            }
        }
        if matched {
            self.0.store.write(state.snapshot()).await?;
        }
        drop(state);
        Ok(())
    }
    /// Canonical terminal deletion revokes all this session's approvals,
    /// including unbound durable records recovered after service restart.
    pub async fn retire_session(&self, session: [u8; 16]) -> BrokerResult<()> {
        self.0.authority.revoke_session(session).map_err(grant_error)?;
        let id = uuid::Uuid::from_bytes(session).to_string();
        let mut state = self.0.state.lock().await;
        let mut matched = false;
        for entry in state
            .grants
            .values_mut()
            .filter(|entry| entry.record.summary.session_id == id)
        {
            matched = true;
            self.revoke_entry(entry)?;
        }
        if matched {
            self.0.store.write(state.snapshot()).await?;
        }
        drop(state);
        Ok(())
    }
    /// Host-only material use under the same admission/fencing authority as
    /// managed retirement. The callback must not reenter the broker and no
    /// HTTP route may return its credential material to the caller.
    pub async fn with_grant<T>(
        &self,
        binding: SessionBinding,
        id: &str,
        target: &crate::GrantTarget,
        policy: impl Fn(&GrantSession, &GrantCapability) -> bool,
        use_token: impl FnOnce(&str) -> T,
    ) -> BrokerResult<T> {
        let (session_id, owner_generation) = binding.ids();
        if binding.expires <= SystemTime::now() {
            return Err(api::CredentialErrorCode::SessionUnavailable);
        }
        let state = self.0.state.lock().await;
        let entry = state.grants.get(id).ok_or(api::CredentialErrorCode::GrantUnavailable)?;
        if entry.record.summary.session_id != session_id
            || entry.record.summary.owner_generation != owner_generation
            || entry.binding != Some(binding.session)
            || entry.record.summary.state != api::GrantState::Active
        {
            return Err(api::CredentialErrorCode::OwnerChanged);
        }
        let handle = entry.handle.ok_or(api::CredentialErrorCode::GrantUnavailable)?;
        drop(state);
        self.0
            .authority
            .with_token(handle, binding.session, target, Instant::now(), policy, use_token)
            .await
            .map_err(grant_error)
    }

    pub async fn create_grant(
        &self,
        binding: SessionBinding,
        mut request: api::CreateSessionGrantRequest,
        policy: impl Fn(&GrantSession, &GrantCapability) -> bool,
    ) -> BrokerResult<api::SessionGrantSummary> {
        uuid(&request.request_id)?;
        uuid(&request.expected_owner_generation)?;
        normalize_grant_request(&mut request, self.0.limits)?;
        let (session_id, owner_generation) = binding.ids();
        if request.expected_owner_generation != owner_generation {
            return Err(api::CredentialErrorCode::OwnerChanged);
        }
        let mut state = self.0.state.lock().await;
        if let Some(id) = state
            .grants
            .values()
            .find(|entry| entry.record.request.request_id == request.request_id)
            .map(|entry| entry.record.summary.id.clone())
        {
            let State {
                connections, grants, ..
            } = &mut *state;
            let entry = grants.get_mut(&id).expect("matched grant");
            if entry.record.request != request
                || entry.record.summary.session_id != session_id
                || entry.record.summary.owner_generation != owner_generation
            {
                return Err(api::CredentialErrorCode::RequestConflict);
            }
            self.synchronize(entry, connections, binding, &policy)?;
            let result = entry.record.summary.clone();
            drop(state);
            return Ok(result);
        }
        if state.grants.len() >= self.0.limits.max_grants {
            return Err(api::CredentialErrorCode::StorageUnavailable);
        }
        let entry = state
            .connections
            .get(&request.connection_id)
            .ok_or(api::CredentialErrorCode::ConnectionUnavailable)?;
        if entry.record.retiring {
            return Err(api::CredentialErrorCode::RevocationPending);
        }
        let connection = entry
            .connection
            .clone()
            .ok_or(api::CredentialErrorCode::ConnectionUnavailable)?;
        let capabilities = self.capabilities(&entry.record.scope_profile_id, &request.capability_ids)?;
        let now = SystemTime::now();
        let default = now + self.0.limits.grant_lifetime;
        let expires = request
            .expires_at
            .as_ref()
            .map(|value| humantime::parse_rfc3339(value).map_err(|_| api::CredentialErrorCode::RequestConflict))
            .transpose()?
            .unwrap_or(default.min(binding.expires));
        if expires > default || expires > binding.expires || expires <= now {
            return Err(api::CredentialErrorCode::SessionUnavailable);
        }
        let consent_generation = connection.authorization().map_err(connection_error)?.generation;
        let handle = self
            .0
            .authority
            .issue_set(
                connection,
                binding.session,
                capabilities,
                expires.duration_since(now).expect("future expiry"),
                Instant::now(),
                &policy,
            )
            .map_err(grant_error)?;
        let id = capsem_crypto::uuid_v4().to_string();
        let summary = api::SessionGrantSummary {
            id: id.clone(),
            session_id,
            owner_generation,
            connection_id: request.connection_id.clone(),
            capability_ids: request.capability_ids.clone(),
            state: api::GrantState::Active,
            revision: 1,
            created_at: timestamp(now)?,
            expires_at: timestamp(expires)?,
            revoked_at: None,
            runtime_bound: true,
        };
        state.grants.insert(
            id.clone(),
            GrantEntry {
                record: GrantRecord {
                    request,
                    summary: summary.clone(),
                    consent_generation,
                },
                handle: Some(handle),
                binding: Some(binding.session),
            },
        );
        if let Err(error) = self.0.store.write(state.snapshot()).await {
            let entry = state.grants.get_mut(&id).expect("new grant");
            self.revoke_entry(entry)?;
            return Err(error);
        }
        drop(state);
        Ok(summary)
    }

    pub async fn grants(
        &self,
        binding: SessionBinding,
        policy: impl Fn(&GrantSession, &GrantCapability) -> bool,
    ) -> BrokerResult<api::SessionGrantListResponse> {
        let (session_id, owner_generation) = binding.ids();
        let mut state = self.0.state.lock().await;
        let State {
            connections, grants, ..
        } = &mut *state;
        let mut result = vec![];
        let mut denied = BTreeSet::new();
        for entry in grants.values_mut().filter(|entry| {
            entry.record.summary.session_id == session_id && entry.record.summary.owner_generation == owner_generation
        }) {
            self.synchronize(entry, connections, binding, &policy)?;
            if entry.record.summary.state == api::GrantState::Active && !entry.record.summary.runtime_bound {
                denied.extend(entry.record.summary.capability_ids.iter().cloned());
            }
            result.push(entry.record.summary.clone());
        }
        drop(state);
        Ok(api::SessionGrantListResponse {
            session_id,
            owner_generation,
            grants: result,
            denied_capability_ids: denied.into_iter().collect(),
        })
    }

    pub async fn revoke_grant(
        &self,
        binding: SessionBinding,
        id: &str,
    ) -> BrokerResult<api::RevokeSessionGrantResponse> {
        self.revoke_approval(binding.owner(), id).await
    }

    pub(super) fn retire_connection_grants(&self, state: &mut State, id: &str) -> BrokerResult<u32> {
        let mut count = 0_u32;
        for entry in state
            .grants
            .values_mut()
            .filter(|entry| entry.record.summary.connection_id == id)
        {
            if entry.record.summary.state == api::GrantState::Active {
                self.revoke_entry(entry)?;
                count += 1;
            }
        }
        Ok(count)
    }

    pub(super) fn revoke_entry(&self, entry: &mut GrantEntry) -> BrokerResult<()> {
        entry.record.summary.runtime_bound = false;
        if let Some(handle) = entry.handle.take() {
            self.0.authority.revoke(handle).map_err(grant_error)?;
        }
        if entry.record.summary.state == api::GrantState::Active {
            entry.record.summary.state = api::GrantState::Revoked;
            entry.record.summary.revision = next_revision(entry.record.summary.revision)?;
            entry.record.summary.revoked_at = Some(timestamp(SystemTime::now())?);
        }
        Ok(())
    }

    fn capabilities(&self, profile: &str, ids: &[String]) -> BrokerResult<Vec<GrantCapability>> {
        let profile = self
            .0
            .profiles
            .get(profile)
            .ok_or(api::CredentialErrorCode::ConfigurationRequired)?;
        if ids.is_empty() || ids.len() > self.0.limits.max_capabilities {
            return Err(api::CredentialErrorCode::UnsupportedCapability);
        }
        let set: BTreeSet<_> = ids.iter().collect();
        if set.len() != ids.len() {
            return Err(api::CredentialErrorCode::UnsupportedCapability);
        }
        ids.iter()
            .map(|id| {
                profile
                    .capabilities
                    .get(id)
                    .cloned()
                    .ok_or(api::CredentialErrorCode::UnsupportedCapability)
            })
            .collect()
    }

    fn synchronize(
        &self,
        entry: &mut GrantEntry,
        connections: &BTreeMap<String, ConnectionEntry>,
        binding: SessionBinding,
        policy: &impl Fn(&GrantSession, &GrantCapability) -> bool,
    ) -> BrokerResult<()> {
        entry.record.summary.runtime_bound = false;
        if entry.record.summary.state != api::GrantState::Active {
            return Ok(());
        }
        let expiry = humantime::parse_rfc3339(&entry.record.summary.expires_at)
            .map_err(|_| api::CredentialErrorCode::StorageUnavailable)?;
        let now = SystemTime::now();
        if expiry <= now {
            self.expire_entry(entry)?;
            return Ok(());
        }
        let Some(connection) = connections.get(&entry.record.summary.connection_id) else {
            self.revoke_entry(entry)?;
            return Ok(());
        };
        let Some(owner) = &connection.connection else {
            return Ok(());
        };
        if connection.record.retiring
            || !owner
                .authorization()
                .is_ok_and(|authorization| authorization.generation == entry.record.consent_generation)
        {
            self.revoke_entry(entry)?;
            return Ok(());
        }
        let capabilities = self.capabilities(
            &connection.record.scope_profile_id,
            &entry.record.summary.capability_ids,
        )?;
        if !capabilities
            .iter()
            .all(|capability| policy(&binding.session, capability))
        {
            return Ok(());
        }
        if let Some(handle) = entry.handle {
            match self.0.authority.visibility(
                handle,
                entry.binding.expect("runtime handle has binding"),
                Instant::now(),
            ) {
                Some(crate::grants::GrantVisibility::Active) if entry.binding == Some(binding.session) => {}
                Some(crate::grants::GrantVisibility::Preserved) => {
                    entry.handle = Some(
                        self.0
                            .authority
                            .rebind(handle, binding.session, Instant::now(), policy)
                            .map_err(grant_error)?,
                    );
                    entry.binding = Some(binding.session);
                }
                Some(crate::grants::GrantVisibility::Active) => return Ok(()),
                _ => {
                    self.revoke_entry(entry)?;
                    return Ok(());
                }
            }
        } else {
            // Only durable explicit approval with the original consent and
            // trusted current same owner may reconstruct runtime authority.
            let lifetime = expiry
                .min(binding.expires)
                .duration_since(now)
                .map_err(|_| api::CredentialErrorCode::SessionUnavailable)?;
            entry.handle = Some(
                self.0
                    .authority
                    .issue_set(
                        owner.clone(),
                        binding.session,
                        capabilities,
                        lifetime,
                        Instant::now(),
                        policy,
                    )
                    .map_err(grant_error)?,
            );
            entry.binding = Some(binding.session);
        }
        entry.record.summary.runtime_bound = true;
        Ok(())
    }
}

pub(super) fn grant_error(error: crate::GrantError) -> api::CredentialErrorCode {
    match error {
        crate::GrantError::Denied => api::CredentialErrorCode::PolicyDenied,
        crate::GrantError::Invalid => api::CredentialErrorCode::UnsupportedCapability,
        crate::GrantError::Capacity | crate::GrantError::Unavailable => api::CredentialErrorCode::GrantUnavailable,
    }
}
