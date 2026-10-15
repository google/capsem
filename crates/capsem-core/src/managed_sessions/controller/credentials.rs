//! Trusted grant admission observes this lifecycle's live original ownership.
use super::*;
use std::time::Instant;

pub struct CredentialOwner {
    controller: std::sync::Weak<Inner>,
    work: std::sync::Weak<Work>,
    ticket: Ticket,
    binding: VmBinding,
}
impl CredentialOwner {
    pub fn generation(&self) -> Uuid {
        self.ticket.generation()
    }
    /// No registry I/O or caller-selected facts. Updated leases and cancellation
    /// remain live, including after a handler awaited broker publication.
    pub fn remaining(&self) -> Option<Duration> {
        self.controller.upgrade()?;
        let work = self.work.upgrade()?;
        if work.recovering || work.cancel.is_cancelled() {
            return None;
        }
        let binding = work.binding.lock().ok()?;
        if binding.as_ref() != Some(&self.binding) {
            return None;
        }
        drop(binding);
        let ticket = work.ticket.lock().ok()?;
        if ticket.as_ref().is_none_or(|ticket| {
            ticket.request() != self.ticket.request() || ticket.generation() != self.ticket.generation()
        }) {
            return None;
        }
        drop(ticket);
        let deadline = (*work.deadline.borrow())?;
        let remaining = deadline.at.checked_duration_since(Instant::now())?;
        if remaining.is_zero() || !work.effects.is_live(&self.binding).ok()? {
            return None;
        }
        Some(remaining)
    }
    pub fn is_current(&self) -> bool {
        self.remaining().is_some()
    }
}

impl Controller {
    /// The service supplies its exact registered native spawn. A matching
    /// managed intent without active authority fails rather than becoming an
    /// ordinary attached-session owner. No ownership capability is exported.
    pub async fn credential_owner(&self, binding: &VmBinding) -> ControlResult<Option<CredentialOwner>> {
        let works = self.0.works.lock().await;
        let mut selected = None;
        for work in works.values() {
            if work.intent.lock().map_err(|_| Failure::RecoveryPending)?.as_ref() == Some(binding) {
                selected = Some(Arc::clone(work));
                break;
            }
        }
        drop(works);
        let Some(work) = selected else {
            return if self.owns_vm(binding).await? {
                Err(Failure::RecoveryPending)
            } else {
                Ok(None)
            };
        };
        let ticket = work
            .ticket
            .lock()
            .map_err(|_| Failure::RecoveryPending)?
            .clone()
            .ok_or(Failure::RecoveryPending)?;
        let snapshot = self.snapshot(ticket.clone()).await?;
        if snapshot.state() != State::Active || snapshot.vm() != Some(binding) {
            return Err(Failure::RecoveryPending);
        }
        let owner = CredentialOwner {
            controller: Arc::downgrade(&self.0),
            work: Arc::downgrade(&work),
            ticket,
            binding: binding.clone(),
        };
        if !owner.is_current() {
            return Err(Failure::Forbidden);
        }
        Ok(Some(owner))
    }
}
