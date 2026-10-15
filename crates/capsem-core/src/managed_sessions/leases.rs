use super::*;
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

#[cfg(test)]
mod tests;

/// Supplied by the service's reviewed policy. There is deliberately no default
/// or client-controlled duration on a renewal operation.
#[derive(Clone, Copy, Debug)]
pub struct LeasePolicy {
    duration_ms: u64,
}

#[derive(Clone, Copy)]
pub(super) enum LeaseLimit {
    Unconfigured,
    Policy(LeasePolicy),
    Denied,
}

pub(super) struct LeaseLimitState {
    limit: LeaseLimit,
    revision: u64,
}

impl Default for LeaseLimitState {
    fn default() -> Self {
        Self {
            limit: LeaseLimit::Unconfigured,
            revision: 0,
        }
    }
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
    pub(super) wall_ms: u64,
    monotonic: Instant,
}

impl LeaseClock {
    pub fn new(wall_ms: u64, monotonic: Instant) -> Self {
        Self { wall_ms, monotonic }
    }

    pub(super) fn now() -> Result<Self> {
        let wall = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        Ok(Self::new(u64::try_from(wall.as_millis())?, Instant::now()))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LeaseFacts {
    pub(super) duration_ms: u64,
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
    revision: u64,
}

#[derive(Clone, Copy)]
pub(super) struct LeaseDeadline {
    pub(super) at: Instant,
    pub(super) revision: u64,
}

impl Registry {
    pub(super) fn runtime_deadline(&self, ticket: &Ticket) -> Result<LeaseDeadline> {
        let deadlines = self.runtime_deadlines()?;
        let runtime = deadlines
            .get(&ticket.request)
            .context("managed runtime deadline is missing")?;
        ensure!(
            runtime.generation == ticket.generation,
            "managed runtime generation mismatch"
        );
        let deadline = LeaseDeadline {
            at: runtime.expires,
            revision: runtime.revision,
        };
        drop(deadlines);
        Ok(deadline)
    }
    /// Initialize once while still Reserved, before begin_create or VM effects.
    pub fn start_lease(&self, ticket: &Ticket, policy: LeasePolicy, clock: LeaseClock) -> Result<Snapshot> {
        let _lease = self.lease()?;
        let mut record = self.ticket_record(ticket)?;
        ensure!(
            record.state == State::Reserved && record.lease.is_none(),
            "managed lease initialization is no longer admissible"
        );
        let (facts, runtime) = lease_deadline(record.generation, self.limited_duration(policy.duration_ms)?, clock, 1)?;
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
        record.authenticate(capability)?;
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
        let revision = deadlines
            .get(&request)
            .context("managed runtime deadline missing")?
            .revision
            .checked_add(1)
            .context("managed lease revision overflow")?;
        let (facts, runtime) = lease_deadline(record.generation, duration, clock, revision)?;
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
        self.narrow_lease(record, deadlines)?;
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

    /// Publish the host limit before inventory or disk mutation can fail.
    /// Queued admission and renewal cannot keep using a permissive old limit.
    pub fn set_lease_limit(&self, policy: LeasePolicy, clock: LeaseClock) -> Result<Vec<Snapshot>> {
        self.publish_lease_limit(policy)?;
        self.apply_lease_limit(clock)
    }

    pub(super) fn publish_lease_limit(&self, policy: LeasePolicy) -> Result<u64> {
        self.publish_limit(LeaseLimit::Policy(policy))
    }

    pub(super) fn apply_lease_limit(&self, clock: LeaseClock) -> Result<Vec<Snapshot>> {
        let _lease = self.lease()?;
        let mut inventory = self.inventory_records()?;
        let mut deadlines = self.runtime_deadlines()?;
        for record in &mut inventory {
            if record.lease.is_some() && matches!(record.state, State::Reserved | State::Creating | State::Active) {
                self.expire_if_due(record, clock, &mut deadlines)?;
            }
        }
        Ok(inventory.iter().map(Snapshot::from).collect())
    }

    fn limited_duration(&self, duration: u64) -> Result<u64> {
        let limit = self
            .lease_limit
            .lock()
            .map_err(|_| anyhow::anyhow!("managed policy lock poisoned"))?;
        match limit.limit {
            LeaseLimit::Unconfigured => Ok(duration),
            LeaseLimit::Policy(policy) => Ok(duration.min(policy.duration_ms)),
            LeaseLimit::Denied => anyhow::bail!("managed lease policy is unavailable"),
        }
    }

    pub fn deny_lease_admission(&self) -> Result<u64> {
        self.publish_limit(LeaseLimit::Denied)
    }

    fn publish_limit(&self, limit: LeaseLimit) -> Result<u64> {
        let mut current = self
            .lease_limit
            .lock()
            .map_err(|_| anyhow::anyhow!("managed policy lock poisoned"))?;
        current.revision = current
            .revision
            .checked_add(1)
            .context("managed policy revision overflow")?;
        current.limit = limit;
        let revision = current.revision;
        drop(current);
        Ok(revision)
    }

    pub(super) fn policy_revision(&self) -> Result<u64> {
        Ok(self
            .lease_limit
            .lock()
            .map_err(|_| anyhow::anyhow!("managed policy lock poisoned"))?
            .revision)
    }

    pub(super) fn admission_denied(&self, revision: u64) -> Result<bool> {
        let current = self
            .lease_limit
            .lock()
            .map_err(|_| anyhow::anyhow!("managed policy lock poisoned"))?;
        Ok(current.revision == revision && matches!(current.limit, LeaseLimit::Denied))
    }

    fn narrow_lease(&self, record: &mut Record, deadlines: &mut HashMap<Uuid, RuntimeLease>) -> Result<()> {
        let facts = record.lease.as_mut().context("managed lease was not initialized")?;
        let duration = self.limited_duration(facts.duration_ms)?;
        if duration == facts.duration_ms {
            return Ok(());
        }
        facts.duration_ms = duration;
        facts.expires_wall_ms = facts
            .last_wall_ms
            .checked_add(duration)
            .context("managed wall deadline overflow")?;
        if let Some(runtime) = deadlines
            .get_mut(&record.request)
            .filter(|runtime| runtime.generation == record.generation)
        {
            runtime.expires = runtime
                .last_monotonic
                .checked_add(Duration::from_millis(duration))
                .context("managed monotonic deadline overflow")?;
            runtime.revision = runtime
                .revision
                .checked_add(1)
                .context("managed lease revision overflow")?;
        }
        self.write(record)
    }
}

fn lease_deadline(
    generation: Uuid,
    duration_ms: u64,
    clock: LeaseClock,
    revision: u64,
) -> Result<(LeaseFacts, RuntimeLease)> {
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
            revision,
        },
    ))
}
