//! Approval ownership needs no runtime, provider refresh or material admission.
use super::*;

#[derive(Debug, Clone, Copy)]
pub struct ApprovalOwner {
    pub(super) session: [u8; 16],
    pub(super) ownership: [u8; 16],
}
impl ApprovalOwner {
    /// The service resolves both identities from its trusted session registry.
    pub fn new(session: [u8; 16], ownership: [u8; 16]) -> BrokerResult<Self> {
        if session == [0; 16] || ownership == [0; 16] {
            return Err(api::CredentialErrorCode::SessionUnavailable);
        }
        Ok(Self { session, ownership })
    }
    pub(super) fn ids(self) -> (String, String) {
        (
            uuid::Uuid::from_bytes(self.session).to_string(),
            uuid::Uuid::from_bytes(self.ownership).to_string(),
        )
    }
}
impl ConnectionBroker {
    /// Bounded metadata inventory. Never issues, rebinds or refreshes authority.
    pub async fn approvals(&self, owner: ApprovalOwner) -> BrokerResult<api::SessionGrantListResponse> {
        let (session_id, owner_generation) = owner.ids();
        let mut state = self.0.state.lock().await;
        let mut result = vec![];
        let mut denied = BTreeSet::new();
        for entry in state.grants.values_mut().filter(|entry| {
            entry.record.summary.session_id == session_id && entry.record.summary.owner_generation == owner_generation
        }) {
            if entry.record.summary.state == api::GrantState::Active {
                let expiry = humantime::parse_rfc3339(&entry.record.summary.expires_at)
                    .map_err(|_| api::CredentialErrorCode::StorageUnavailable)?;
                if expiry <= SystemTime::now() {
                    self.expire_entry(entry)?;
                } else {
                    let bound = match (entry.handle, entry.binding) {
                        (Some(handle), Some(binding)) => {
                            match self.0.authority.visibility(handle, binding, Instant::now()) {
                                Some(crate::grants::GrantVisibility::Active) => true,
                                Some(crate::grants::GrantVisibility::Preserved) => false,
                                Some(crate::grants::GrantVisibility::Expired) => {
                                    self.expire_entry(entry)?;
                                    false
                                }
                                None => {
                                    self.revoke_entry(entry)?;
                                    false
                                }
                            }
                        }
                        (Some(_), None) => return Err(api::CredentialErrorCode::StorageUnavailable),
                        (None, _) => false,
                    };
                    if entry.record.summary.runtime_bound != bound {
                        entry.record.summary.runtime_bound = bound;
                        entry.record.summary.revision = next_revision(entry.record.summary.revision)?;
                    }
                }
            }
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

    /// Revocation fences the existing handle and durably publishes the result;
    /// the owner cannot select a replacement runtime or create new authority.
    pub async fn revoke_approval(
        &self,
        owner: ApprovalOwner,
        id: &str,
    ) -> BrokerResult<api::RevokeSessionGrantResponse> {
        let (session_id, owner_generation) = owner.ids();
        let mut state = self.0.state.lock().await;
        let entry = state
            .grants
            .get_mut(id)
            .ok_or(api::CredentialErrorCode::GrantUnavailable)?;
        if entry.record.summary.session_id != session_id || entry.record.summary.owner_generation != owner_generation {
            return Err(api::CredentialErrorCode::OwnerChanged);
        }
        self.revoke_entry(entry)?;
        let result = api::RevokeSessionGrantResponse {
            grant: entry.record.summary.clone(),
            local_authority: api::RevokedCredentialAuthority::Revoked,
        };
        self.0.store.write(state.snapshot()).await?;
        drop(state);
        Ok(result)
    }

    pub(super) fn expire_entry(&self, entry: &mut GrantEntry) -> BrokerResult<()> {
        if let Some(handle) = entry.handle.take() {
            self.0.authority.revoke(handle).map_err(sessions::grant_error)?;
        }
        entry.record.summary.runtime_bound = false;
        entry.record.summary.state = api::GrantState::Expired;
        entry.record.summary.revision = next_revision(entry.record.summary.revision)?;
        Ok(())
    }
}
