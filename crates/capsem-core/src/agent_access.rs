//! Bounded service-epoch delegated authority. The ledger owns durable history;
//! it never restores this live authority after service restart.
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, Weak},
    time::{Duration, Instant},
};

use capsem_api::{AgentAccessScope, AgentOperation};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::managed_sessions::{
    Capability, CapabilityLease, CapabilityPurpose, CapabilityVerifier, LeaseClock, LeasePolicy,
};

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum Failure {
    #[error("delegated capability denied")]
    Forbidden,
    #[error("delegated admission awaits durable audit")]
    Pending,
    #[error("delegated capability is closed")]
    Closed,
    #[error("delegated request intent differs from its original admission")]
    Conflict,
    #[error("delegated authority capacity exhausted")]
    Capacity,
    #[error("delegated authority unavailable")]
    Unavailable,
    #[error("invalid delegated admission")]
    Invalid,
    #[error("delegated access identity not found")]
    NotFound,
}

pub struct Registry {
    epoch: Uuid,
    inner: Arc<Mutex<Index>>,
}
struct Index {
    max_active: usize,
    max_issued: usize,
    entries: HashMap<Uuid, Entry>,
}
struct Entry {
    verifier: CapabilityVerifier,
    state: State,
}
enum State {
    Pending {
        scope: AgentAccessScope,
        duration: Duration,
        lease: CapabilityLease,
    },
    Active {
        grant: Arc<Grant>,
        duration: Duration,
    },
    Closed,
}

pub enum Reservation {
    New(Admission),
    Existing(Arc<Grant>),
}

/// A caller holds this only while durably auditing issuance. Dropping it on
/// error or cancellation closes the reservation rather than leaking authority.
pub struct Admission {
    id: Uuid,
    registry: Weak<Mutex<Index>>,
}

/// Nonsecret authenticated scope. Cancellation and finite lease remain live
/// after a handler has extracted this proof.
pub struct Grant {
    id: Uuid,
    scope: AgentAccessScope,
    lease: Mutex<CapabilityLease>,
    expires_wall_ms: u64,
    deadline: Instant,
    cancel: CancellationToken,
}

impl Registry {
    pub fn new(max_active: usize, max_issued: usize) -> Result<Self, Failure> {
        if max_active == 0
            || max_active > max_issued
            || max_active > capsem_proto::AGENT_ACCESS_MAX_ACTIVE
            || max_issued > capsem_proto::AGENT_ACCESS_MAX_ISSUED_PER_EPOCH
        {
            return Err(Failure::Invalid);
        }
        Ok(Self {
            epoch: capsem_crypto::uuid_v4(),
            inner: Arc::new(Mutex::new(Index {
                max_active,
                max_issued,
                entries: HashMap::new(),
            })),
        })
    }

    pub fn epoch(&self) -> Uuid {
        self.epoch
    }

    /// Sample clocks under the owning locks. Sampling before waiting for a lock
    /// can mistake concurrent request ordering for clock rollback.
    pub fn reserve(
        &self,
        id: Uuid,
        capability: &Capability,
        scope: AgentAccessScope,
        duration: Duration,
        mut sample: impl FnMut() -> Option<LeaseClock>,
    ) -> Result<Reservation, Failure> {
        if id.get_version_num() != 4
            || duration.is_zero()
            || duration > Duration::from_secs(capsem_proto::AGENT_ACCESS_MAX_LEASE_SECONDS as u64)
        {
            return Err(Failure::Invalid);
        }
        let mut index = self.inner.lock().map_err(|_| Failure::Unavailable)?;
        index.expire(&mut sample);
        if let Some(entry) = index.entries.get(&id) {
            if !entry.verifier.authenticate(id, capability) {
                return Err(Failure::Forbidden);
            }
            return match &entry.state {
                State::Pending { .. } => Err(Failure::Pending),
                State::Closed => Err(Failure::Closed),
                State::Active {
                    grant,
                    duration: original,
                } => {
                    if *original != duration || grant.scope != scope {
                        Err(Failure::Conflict)
                    } else {
                        Ok(Reservation::Existing(Arc::clone(grant)))
                    }
                }
            };
        }
        let active = index
            .entries
            .values()
            .filter(|entry| !matches!(entry.state, State::Closed))
            .count();
        if active >= index.max_active || index.entries.len() >= index.max_issued {
            return Err(Failure::Capacity);
        }
        let clock = sample().ok_or(Failure::Unavailable)?;
        let lease = LeasePolicy::new(duration)
            .map_err(|_| Failure::Invalid)?
            .issue(clock)
            .map_err(|_| Failure::Invalid)?;
        index.entries.insert(
            id,
            Entry {
                verifier: capability.verifier(id, CapabilityPurpose::DelegatedAgent),
                state: State::Pending { scope, duration, lease },
            },
        );
        drop(index);
        Ok(Reservation::New(Admission {
            id,
            registry: Arc::downgrade(&self.inner),
        }))
    }

    pub fn authenticate(
        &self,
        id: Uuid,
        capability: &Capability,
        sample: impl FnMut() -> Option<LeaseClock>,
    ) -> Result<Arc<Grant>, Failure> {
        let mut index = self.inner.lock().map_err(|_| Failure::Unavailable)?;
        let entry = index.entries.get_mut(&id).ok_or(Failure::Forbidden)?;
        if !entry.verifier.authenticate(id, capability) {
            return Err(Failure::Forbidden);
        }
        let result = match &entry.state {
            State::Active { grant, .. } if grant.is_live(sample) => Ok(Arc::clone(grant)),
            State::Active { .. } => {
                entry.state = State::Closed;
                Err(Failure::Closed)
            }
            State::Pending { .. } => Err(Failure::Pending),
            State::Closed => Err(Failure::Closed),
        };
        drop(index);
        result
    }

    /// Recognize a revoked credential only for refusal attribution. This
    /// deliberately returns no grant and cannot authorize a request.
    pub fn recognizes(&self, id: Uuid, capability: &Capability) -> Result<bool, Failure> {
        let index = self.inner.lock().map_err(|_| Failure::Unavailable)?;
        Ok(index
            .entries
            .get(&id)
            .is_some_and(|entry| entry.verifier.authenticate(id, capability)))
    }

    pub fn revoke(&self, id: Uuid) -> Result<bool, Failure> {
        let mut index = self.inner.lock().map_err(|_| Failure::Unavailable)?;
        let entry = index.entries.get_mut(&id).ok_or(Failure::NotFound)?;
        if let State::Active { grant, .. } = &entry.state {
            grant.cancel.cancel();
        }
        let changed = !matches!(entry.state, State::Closed);
        entry.state = State::Closed;
        drop(index);
        Ok(changed)
    }

    pub fn expire(&self, mut sample: impl FnMut() -> Option<LeaseClock>) -> Result<(), Failure> {
        self.inner.lock().map_err(|_| Failure::Unavailable)?.expire(&mut sample);
        Ok(())
    }
}

impl Index {
    fn expire(&mut self, sample: &mut impl FnMut() -> Option<LeaseClock>) {
        for entry in self.entries.values_mut() {
            let live = match &mut entry.state {
                State::Pending { lease, .. } => sample().is_some_and(|clock| lease.is_live(clock)),
                State::Active { grant, .. } => grant.is_live(&mut *sample),
                State::Closed => continue,
            };
            if !live {
                entry.state = State::Closed;
            }
        }
    }
}

impl Drop for Index {
    fn drop(&mut self) {
        for entry in self.entries.values() {
            if let State::Active { grant, .. } = &entry.state {
                grant.cancel.cancel();
            }
        }
    }
}

impl Admission {
    pub fn commit(self, mut sample: impl FnMut() -> Option<LeaseClock>) -> Result<Arc<Grant>, Failure> {
        let registry = self.registry.upgrade().ok_or(Failure::Unavailable)?;
        let mut index = registry.lock().map_err(|_| Failure::Unavailable)?;
        let entry = index.entries.get_mut(&self.id).ok_or(Failure::Closed)?;
        let State::Pending {
            scope,
            duration,
            mut lease,
        } = std::mem::replace(&mut entry.state, State::Closed)
        else {
            return Err(Failure::Closed);
        };
        if !sample().is_some_and(|clock| lease.is_live(clock)) {
            return Err(Failure::Closed);
        }
        let grant = Arc::new(Grant {
            id: self.id,
            scope,
            expires_wall_ms: lease.expires_wall_ms(),
            deadline: lease.deadline(),
            lease: Mutex::new(lease),
            cancel: CancellationToken::new(),
        });
        entry.state = State::Active {
            grant: Arc::clone(&grant),
            duration,
        };
        // Release the lock before this admission's cancellation-safe Drop.
        drop(index);
        Ok(grant)
    }
}

impl Drop for Admission {
    fn drop(&mut self) {
        if let Some(registry) = self.registry.upgrade() {
            if let Ok(mut index) = registry.lock() {
                if let Some(entry) = index.entries.get_mut(&self.id) {
                    if matches!(entry.state, State::Pending { .. }) {
                        entry.state = State::Closed;
                    }
                }
            }
        }
    }
}

impl Grant {
    pub fn id(&self) -> Uuid {
        self.id
    }
    pub fn scope(&self) -> &AgentAccessScope {
        &self.scope
    }
    pub fn expires_wall_ms(&self) -> u64 {
        self.expires_wall_ms
    }
    pub fn deadline(&self) -> Instant {
        self.deadline
    }
    pub async fn revoked(&self) {
        self.cancel.cancelled().await;
    }

    pub fn is_live(&self, mut sample: impl FnMut() -> Option<LeaseClock>) -> bool {
        if self.cancel.is_cancelled() {
            return false;
        }
        let live = self
            .lease
            .lock()
            .ok()
            .is_some_and(|mut lease| sample().is_some_and(|clock| lease.is_live(clock)));
        if !live {
            self.cancel.cancel();
        }
        live && !self.cancel.is_cancelled()
    }

    pub fn authorizes_operation(&self, operation: AgentOperation, sample: impl FnMut() -> Option<LeaseClock>) -> bool {
        self.is_live(sample) && self.scope.permits(operation)
    }

    pub fn authorizes_session(
        &self,
        operation: AgentOperation,
        session_id: &str,
        generation: &str,
        sample: impl FnMut() -> Option<LeaseClock>,
    ) -> bool {
        self.authorizes_operation(operation, sample) && self.scope.permits_session(session_id, generation)
    }
}

#[cfg(test)]
mod tests;
