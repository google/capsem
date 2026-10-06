use super::*;
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

/// Supplied by the service's reviewed policy. There is deliberately no default
/// or client-controlled duration on a renewal operation.
#[derive(Clone, Copy, Debug)]
pub struct LeasePolicy {
    duration_ms: u64,
}

impl LeasePolicy {
    pub fn new(duration: Duration) -> Result<Self> {
        let duration_ms = u64::try_from(duration.as_millis()).context("managed lease duration overflow")?;
        ensure!(duration_ms > 0, "managed lease must have a finite positive duration");
        Ok(Self { duration_ms })
    }
}

/// Sampled by the trusted service, never from an HTTP request body.
#[derive(Clone, Copy, Debug)]
pub struct LeaseClock {
    wall_ms: u64,
    monotonic: Instant,
}

impl LeaseClock {
    pub fn new(wall_ms: u64, monotonic: Instant) -> Self {
        Self { wall_ms, monotonic }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LeaseFacts {
    duration_ms: u64,
    last_wall_ms: u64,
    pub(super) expires_wall_ms: u64,
}

impl LeaseFacts {
    pub(super) fn validate(&self) -> Result<()> {
        ensure!(
            self.duration_ms > 0 && self.last_wall_ms.checked_add(self.duration_ms) == Some(self.expires_wall_ms),
            "invalid managed lease facts"
        );
        Ok(())
    }
}

pub(super) struct RuntimeLease {
    generation: Uuid,
    last_monotonic: Instant,
    expires: Instant,
}

impl Registry {
    /// Initialize once while still Reserved, before begin_create or VM effects.
    pub fn start_lease(&self, ticket: &Ticket, policy: LeasePolicy, clock: LeaseClock) -> Result<Snapshot> {
        let _lease = self.lease()?;
        let mut record = self.ticket_record(ticket)?;
        ensure!(
            record.state == State::Reserved && record.lease.is_none(),
            "managed lease initialization is no longer admissible"
        );
        let (facts, runtime) = lease_deadline(record.generation, policy.duration_ms, clock)?;
        record.lease = Some(facts);
        let mut deadlines = self.runtime_deadlines()?;
        self.write(&record)?;
        deadlines.insert(record.request, runtime);
        drop(deadlines);
        Ok(Snapshot::from(&record))
    }

    /// Renew an existing live lease, or return the conservative terminal/close
    /// state. A newly opened Registry has no monotonic authority to extend a
    /// previous process's lifetime, even when the wall clock suggests time left.
    pub fn renew_lease(&self, request: Uuid, capability: &Capability, clock: LeaseClock) -> Result<Snapshot> {
        let _lease = self.lease()?;
        let mut record = self.read(request)?.context("managed reservation is missing")?;
        ensure!(
            blake3::Hash::from(record.capability_hash) == capability.hash(request),
            "managed capability mismatch"
        );
        let mut deadlines = self.runtime_deadlines()?;
        if !matches!(record.state, State::Reserved | State::Creating | State::Active) {
            deadlines.remove(&request);
            return Ok(Snapshot::from(&record));
        }
        if self.expire_if_due(&mut record, clock, &mut deadlines)? {
            return Ok(Snapshot::from(&record));
        }
        let duration = record
            .lease
            .as_ref()
            .context("managed lease was not initialized")?
            .duration_ms;
        let (facts, runtime) = lease_deadline(record.generation, duration, clock)?;
        record.lease = Some(facts);
        self.write(&record)?;
        deadlines.insert(request, runtime);
        drop(deadlines);
        Ok(Snapshot::from(&record))
    }

    /// Service reapers check each owned request without requiring a client.
    pub fn expire_lease(&self, ticket: &Ticket, clock: LeaseClock) -> Result<Snapshot> {
        let _lease = self.lease()?;
        let mut record = self.ticket_record(ticket)?;
        let mut deadlines = self.runtime_deadlines()?;
        if matches!(record.state, State::Closing | State::Closed) {
            deadlines.remove(&record.request);
        } else {
            self.expire_if_due(&mut record, clock, &mut deadlines)?;
        }
        drop(deadlines);
        Ok(Snapshot::from(&record))
    }

    pub(super) fn expire_if_due(
        &self,
        record: &mut Record,
        clock: LeaseClock,
        deadlines: &mut HashMap<Uuid, RuntimeLease>,
    ) -> Result<bool> {
        let facts = record.lease.as_ref().context("managed lease was not initialized")?;
        let expired = record.state == State::Unknown
            || clock.wall_ms < facts.last_wall_ms
            || clock.wall_ms >= facts.expires_wall_ms
            || deadlines.get(&record.request).is_none_or(|runtime| {
                runtime.generation != record.generation
                    || clock.monotonic < runtime.last_monotonic
                    || clock.monotonic >= runtime.expires
            });
        if expired {
            record.state = if record.state == State::Reserved {
                State::Closed
            } else {
                State::Closing
            };
            self.write(record)?;
            deadlines.remove(&record.request);
        }
        Ok(expired)
    }
}

fn lease_deadline(generation: Uuid, duration_ms: u64, clock: LeaseClock) -> Result<(LeaseFacts, RuntimeLease)> {
    let expires_wall_ms = clock
        .wall_ms
        .checked_add(duration_ms)
        .context("managed wall deadline overflow")?;
    let expires = clock
        .monotonic
        .checked_add(Duration::from_millis(duration_ms))
        .context("managed monotonic deadline overflow")?;
    Ok((
        LeaseFacts {
            duration_ms,
            last_wall_ms: clock.wall_ms,
            expires_wall_ms,
        },
        RuntimeLease {
            generation,
            last_monotonic: clock.monotonic,
            expires,
        },
    ))
}
