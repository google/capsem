//! Coalescing of identical in-flight upstream lookups.
//!
//! When many guest processes ask for the same name at once (a build fetching
//! from one registry, an agent fanning out requests), every miss used to go
//! upstream on its own: two hundred concurrent queries for one name were two
//! hundred upstream datagrams, two hundred timeouts when the upstream was
//! slow, and two hundred sockets. Now the first query for a
//! `(qname, qtype, qclass)` leads the upstream lookup and the rest wait for
//! its checked answer.
//!
//! Where this sits matters for policy: the handler joins a query to a leader
//! only after that query's own security evaluation, local fixtures, redirect
//! check and cache lookup have all fallen through to the upstream path. A
//! blocked query is answered long before it could join anything, so queries
//! with different policy outcomes are never merged. Every follower still gets
//! its own transaction id patched in and its own ledger row.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::oneshot;

/// The identity of one lookup. Two queries that differ in any of these are
/// different lookups.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub(super) struct LookupKey {
    pub(super) qname: String,
    pub(super) qtype: u16,
    pub(super) qclass: u16,
}

/// What the leader's upstream lookup produced: the checked answer bytes and
/// how long the upstream took, or the error text.
pub(super) type UpstreamOutcome = Result<(Vec<u8>, Duration), String>;

type Waiters = Vec<oneshot::Sender<Arc<UpstreamOutcome>>>;

/// Lookups currently on their way upstream, by key.
#[derive(Default)]
pub(super) struct InFlightLookups {
    pending: Mutex<HashMap<LookupKey, Waiters>>,
}

/// What a query does after asking to join.
pub(super) enum Role {
    /// Nobody is looking this up: do it, then `finish` the lease.
    Lead(LeaderLease),
    /// Someone is: wait for their outcome.
    Follow(oneshot::Receiver<Arc<UpstreamOutcome>>),
}

/// The leader's obligation to report back. Dropping it without `finish`
/// (the leading task was cancelled) fails every follower instead of leaving
/// them waiting on an answer that will never come.
pub(super) struct LeaderLease {
    key: Option<LookupKey>,
    lookups: Arc<InFlightLookups>,
}

impl InFlightLookups {
    pub(super) fn join_or_lead(self: &Arc<Self>, key: LookupKey) -> Role {
        let (tx, rx) = oneshot::channel();
        let joined = {
            let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
            match pending.get_mut(&key) {
                Some(waiters) => {
                    waiters.push(tx);
                    true
                }
                None => {
                    pending.insert(key.clone(), Vec::new());
                    false
                }
            }
        };
        if joined {
            return Role::Follow(rx);
        }
        Role::Lead(LeaderLease {
            key: Some(key),
            lookups: Arc::clone(self),
        })
    }

    /// Lookups in flight right now.
    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.pending.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    fn settle(&self, key: &LookupKey, outcome: Arc<UpstreamOutcome>) {
        let waiters = self
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(key)
            .unwrap_or_default();
        for waiter in waiters {
            // channel-closed-ok: a follower that gave up (its own deadline)
            // is not owed an answer.
            let _ = waiter.send(Arc::clone(&outcome));
        }
    }
}

impl LeaderLease {
    /// Hand the outcome to every follower and release the key.
    pub(super) fn finish(mut self, outcome: UpstreamOutcome) -> Arc<UpstreamOutcome> {
        let outcome = Arc::new(outcome);
        if let Some(key) = self.key.take() {
            self.lookups.settle(&key, Arc::clone(&outcome));
        }
        outcome
    }
}

impl Drop for LeaderLease {
    fn drop(&mut self) {
        if let Some(key) = self.key.take() {
            self.lookups.settle(
                &key,
                Arc::new(Err("upstream lookup abandoned by its leader".to_string())),
            );
        }
    }
}

#[cfg(test)]
mod tests;
