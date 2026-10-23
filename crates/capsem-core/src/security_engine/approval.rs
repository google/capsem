//! Live operation continuations, distinct from the append-only approval ledger.
//! No policy lock or request payload is retained while a human decides.
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use capsem_proto::approval::{ApprovalOutcome, ApprovalRequest};
use tokio::sync::{oneshot, watch};
use tokio::time::Instant;
use uuid::Uuid;

pub mod client;
pub(crate) mod enforcement;
pub use enforcement::resolve_emission;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ApprovalBinding {
    pub session_id: String,
    pub generation: Uuid,
}

#[derive(Debug, Clone, Copy)]
pub struct ApprovalLimits {
    pub timeout: Duration,
    pub per_session: usize,
    pub total: usize,
    pub held_bytes: usize,
}

impl Default for ApprovalLimits {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(capsem_proto::APPROVAL_TIMEOUT_SECONDS as u64),
            per_session: capsem_proto::APPROVAL_PENDING_PER_SESSION,
            total: capsem_proto::APPROVAL_PENDING_TOTAL,
            held_bytes: capsem_proto::APPROVAL_HELD_BYTES_TOTAL,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalError {
    Invalid,
    Capacity,
    Missing,
    AlreadyResolving,
}

type Key = (ApprovalBinding, String);

struct Entry {
    published: bool,
    identity: Arc<()>,
    request: ApprovalRequest,
    created_ms: i64,
    deadline: Instant,
    expires_ms: i64,
    delivery: Option<oneshot::Sender<ApprovalOutcome>>,
    cancel: watch::Sender<bool>,
    audit: Arc<tokio::sync::Mutex<()>>,
}

#[derive(Default)]
struct State {
    entries: HashMap<Key, Entry>,
    held_bytes: usize,
}

#[derive(Clone)]
pub struct ApprovalBroker {
    state: Arc<Mutex<State>>,
    limits: ApprovalLimits,
    changes: watch::Sender<u64>,
}

impl Default for ApprovalBroker {
    fn default() -> Self {
        Self::new(ApprovalLimits::default()).expect("valid product approval limits")
    }
}

impl ApprovalBroker {
    pub fn new(limits: ApprovalLimits) -> Result<Self, ApprovalError> {
        let product = ApprovalLimits::default();
        if limits.timeout.is_zero()
            || limits.timeout > product.timeout
            || limits.per_session == 0
            || limits.per_session > product.per_session
            || limits.total == 0
            || limits.total > product.total
            || limits.held_bytes == 0
            || limits.held_bytes > product.held_bytes
        {
            return Err(ApprovalError::Invalid);
        }
        Ok(Self {
            state: Arc::default(),
            limits,
            changes: watch::channel(0).0,
        })
    }

    /// Live queue revisions contain no payloads and are not ledger data.
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }

    fn changed(&self) {
        self.changes
            .send_modify(|revision| *revision = revision.wrapping_add(1));
    }

    pub fn register(
        &self,
        binding: ApprovalBinding,
        request: ApprovalRequest,
    ) -> Result<PendingApproval, ApprovalError> {
        self.register_with_visibility(binding, request, true)
    }

    /// Capacity is reserved before bounded ledger verification. It cannot be
    /// listed or resolved until its owning continuation publishes it.
    pub fn reserve(
        &self,
        binding: ApprovalBinding,
        request: ApprovalRequest,
    ) -> Result<PendingApproval, ApprovalError> {
        self.register_with_visibility(binding, request, false)
    }

    fn register_with_visibility(
        &self,
        binding: ApprovalBinding,
        request: ApprovalRequest,
        published: bool,
    ) -> Result<PendingApproval, ApprovalError> {
        if binding.session_id.is_empty() || binding.session_id.len() > 64 || binding.generation.is_nil() {
            return Err(ApprovalError::Invalid);
        }
        request.validate().map_err(|_| ApprovalError::Invalid)?;
        let key = (binding, request.ask_id.clone());
        let mut state = self.state.lock().unwrap();
        if state.entries.contains_key(&key) {
            return Err(ApprovalError::AlreadyResolving);
        }
        let bytes = usize::try_from(request.held_bytes).map_err(|_| ApprovalError::Invalid)?;
        let total_bytes = state.held_bytes.checked_add(bytes).ok_or(ApprovalError::Capacity)?;
        if state.entries.len() >= self.limits.total
            || total_bytes > self.limits.held_bytes
            || state.entries.keys().filter(|(owner, _)| owner == &key.0).count() >= self.limits.per_session
        {
            return Err(ApprovalError::Capacity);
        }
        let created_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|duration| i64::try_from(duration.as_millis()).ok())
            .ok_or(ApprovalError::Invalid)?;
        let expires_ms = created_ms
            .checked_add(self.limits.timeout.as_millis() as i64)
            .ok_or(ApprovalError::Invalid)?;
        let deadline = Instant::now() + self.limits.timeout;
        let (sender, delivery) = oneshot::channel();
        let (cancel, cancelled) = watch::channel(false);
        let audit = Arc::new(tokio::sync::Mutex::new(()));
        let identity = Arc::new(());
        state.held_bytes = total_bytes;
        state.entries.insert(
            key.clone(),
            Entry {
                published,
                identity: Arc::clone(&identity),
                request,
                created_ms,
                deadline,
                expires_ms,
                delivery: Some(sender),
                cancel: cancel.clone(),
                audit: Arc::clone(&audit),
            },
        );
        drop(state);
        if published {
            self.changed();
        }
        Ok(PendingApproval {
            broker: self.clone(),
            key,
            identity,
            delivery,
            cancel: cancelled,
            deadline,
            audit,
            _cancel_keepalive: cancel,
        })
    }

    /// Reserve a single resolver. The ticket retains the entry's capacity
    /// until audit and delivery finish, and must never be held over human wait.
    pub fn claim(&self, binding: &ApprovalBinding, ask_id: &str) -> Result<ApprovalTicket, ApprovalError> {
        let key = (binding.clone(), ask_id.to_string());
        let mut state = self.state.lock().unwrap();
        let entry = state.entries.get_mut(&key).ok_or(ApprovalError::Missing)?;
        if !entry.published || entry.deadline <= Instant::now() || *entry.cancel.borrow() {
            return Err(ApprovalError::Missing);
        }
        let delivery = entry.delivery.take().ok_or(ApprovalError::AlreadyResolving)?;
        let identity = Arc::clone(&entry.identity);
        let audit = Arc::clone(&entry.audit);
        drop(state);
        self.changed();
        Ok(ApprovalTicket {
            broker: self.clone(),
            key,
            identity,
            delivery: Some(delivery),
            audit,
        })
    }

    pub fn pending_count(&self, binding: &ApprovalBinding) -> usize {
        self.state
            .lock()
            .unwrap()
            .entries
            .iter()
            .filter(|((owner, _), entry)| {
                owner == binding && entry.published && entry.delivery.is_some() && entry.deadline > Instant::now()
            })
            .count()
    }

    pub fn active(&self, binding: &ApprovalBinding, ask_id: &str) -> Option<ApprovalSnapshot> {
        self.state
            .lock()
            .unwrap()
            .entries
            .get(&(binding.clone(), ask_id.to_string()))
            .filter(|entry| entry.published && entry.deadline > Instant::now() && !*entry.cancel.borrow())
            .map(|entry| ApprovalSnapshot {
                request: entry.request.clone(),
                created_ms: entry.created_ms,
                expires_ms: entry.expires_ms,
            })
    }

    /// Live continuation metadata, distinct from stored event history. A
    /// bounded history page must never hide an actionable original operation.
    pub fn active_requests(&self, binding: &ApprovalBinding) -> Vec<ApprovalSnapshot> {
        self.state
            .lock()
            .unwrap()
            .entries
            .iter()
            .filter(|((owner, _), entry)| {
                owner == binding
                    && entry.published
                    && entry.delivery.is_some()
                    && entry.deadline > Instant::now()
                    && !*entry.cancel.borrow()
            })
            .map(|(_, entry)| ApprovalSnapshot {
                request: entry.request.clone(),
                created_ms: entry.created_ms,
                expires_ms: entry.expires_ms,
            })
            .collect()
    }

    pub fn cancel(&self, binding: &ApprovalBinding, ask_id: &str) {
        self.remove(&(binding.clone(), ask_id.to_string()));
    }

    pub fn cancel_generation(&self, binding: &ApprovalBinding) {
        let mut state = self.state.lock().unwrap();
        let keys: Vec<_> = state
            .entries
            .keys()
            .filter(|(owner, _)| owner == binding)
            .cloned()
            .collect();
        let changed = !keys.is_empty();
        for key in keys {
            remove(&mut state, &key);
        }
        drop(state);
        if changed {
            self.changed();
        }
    }

    fn remove(&self, key: &Key) {
        let changed = remove(&mut self.state.lock().unwrap(), key);
        if changed {
            self.changed();
        }
    }

    fn remove_identity(&self, key: &Key, identity: &Arc<()>) {
        let mut state = self.state.lock().unwrap();
        let changed = if state
            .entries
            .get(key)
            .is_some_and(|entry| Arc::ptr_eq(&entry.identity, identity))
        {
            remove(&mut state, key)
        } else {
            false
        };
        drop(state);
        if changed {
            self.changed();
        }
    }
}

fn remove(state: &mut State, key: &Key) -> bool {
    if let Some(entry) = state.entries.remove(key) {
        state.held_bytes -= entry.request.held_bytes as usize;
        let _ = entry.cancel.send(true);
        true
    } else {
        false
    }
}

#[derive(Debug, Clone)]
pub struct ApprovalSnapshot {
    pub request: ApprovalRequest,
    pub created_ms: i64,
    pub expires_ms: i64,
}

pub struct PendingApproval {
    broker: ApprovalBroker,
    key: Key,
    identity: Arc<()>,
    delivery: oneshot::Receiver<ApprovalOutcome>,
    cancel: watch::Receiver<bool>,
    _cancel_keepalive: watch::Sender<bool>,
    deadline: Instant,
    /// Serializes only bounded audit I/O, never the human decision wait.
    pub audit: Arc<tokio::sync::Mutex<()>>,
}

impl PendingApproval {
    /// Publish only after the trusted owner verifies the canonical pending
    /// event. Cancellation and deadline still win during verification.
    pub fn publish(&self) -> bool {
        let mut state = self.broker.state.lock().unwrap();
        let Some(entry) = state.entries.get_mut(&self.key) else {
            return false;
        };
        if !Arc::ptr_eq(&entry.identity, &self.identity) || entry.deadline <= Instant::now() || *entry.cancel.borrow() {
            return false;
        }
        entry.published = true;
        drop(state);
        self.broker.changed();
        true
    }

    pub async fn wait(mut self) -> ApprovalOutcome {
        let outcome = tokio::select! {
            biased;
            _ = self.cancel.changed() => ApprovalOutcome::Cancelled,
            outcome = &mut self.delivery => outcome.unwrap_or(ApprovalOutcome::Unavailable),
            _ = tokio::time::sleep_until(self.deadline) => ApprovalOutcome::Expired,
        };
        if outcome.is_allowed() && Instant::now() >= self.deadline {
            ApprovalOutcome::Expired
        } else {
            outcome
        }
    }
}

impl Drop for PendingApproval {
    fn drop(&mut self) {
        self.broker.remove_identity(&self.key, &self.identity);
    }
}

pub struct ApprovalTicket {
    broker: ApprovalBroker,
    key: Key,
    identity: Arc<()>,
    delivery: Option<oneshot::Sender<ApprovalOutcome>>,
    pub audit: Arc<tokio::sync::Mutex<()>>,
}

impl ApprovalTicket {
    /// The trusted caller has committed and flushed the resolution. Expiry,
    /// cancellation, or runtime retirement still wins before actual delivery.
    pub fn complete_after_audit(mut self, outcome: ApprovalOutcome) -> bool {
        let mut state = self.broker.state.lock().unwrap();
        let live = state.entries.get(&self.key).is_some_and(|entry| {
            entry.published
                && Arc::ptr_eq(&entry.identity, &self.identity)
                && entry.deadline > Instant::now()
                && !*entry.cancel.borrow()
        });
        if !live || !matches!(outcome, ApprovalOutcome::Approved | ApprovalOutcome::Denied) {
            return false;
        }
        // Remove without cancellation: a committed decision is the one wakeup.
        if let Some(entry) = state.entries.remove(&self.key) {
            state.held_bytes -= entry.request.held_bytes as usize;
        }
        drop(state);
        self.broker.changed();
        self.delivery.take().is_some_and(|sender| sender.send(outcome).is_ok())
    }
}

impl Drop for ApprovalTicket {
    fn drop(&mut self) {
        if self.delivery.is_some() {
            self.broker.remove_identity(&self.key, &self.identity);
        }
    }
}

#[cfg(test)]
mod tests;
