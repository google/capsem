//! Completion-only replay-record retirement through the private store owner.

use super::*;
use std::time::Duration;

impl Registry {
    /// Hold ownership admission across the reference inventory and receipt
    /// deletion. A separately sampled inventory races newly prepared spawns.
    pub fn prune_native_retirements(&self, store: &crate::session::spawn_owner::RetirementStore) -> Result<usize> {
        let _lease = self.lease()?;
        let references = self
            .inventory_records()?
            .into_iter()
            .flat_map(|record| record.vm.into_iter().chain(record.spawn_intent))
            .collect::<Vec<_>>();
        store.prune_unreferenced(&references)
    }

    pub fn prune_closed(&self, retention: Duration, clock: LeaseClock) -> Result<usize> {
        let retention_ms = u64::try_from(retention.as_millis()).context("managed retention duration overflow")?;
        ensure!(retention_ms > 0, "managed retention must be positive");
        let _lease = self.lease()?;
        let inventory = self.inventory_records()?;
        let mut removed = 0;
        for mut record in inventory {
            if record.state != State::Closed {
                continue;
            }
            let Some(completed) = record.closed_wall_ms else {
                // Old records cannot prove their completion time. Give them
                // a full window starting with the first trusted observation.
                record.closed_wall_ms = Some(clock.wall_ms);
                self.write(&record)?;
                continue;
            };
            if clock
                .wall_ms
                .checked_sub(completed)
                .is_some_and(|age| age >= retention_ms)
            {
                self.root
                    .remove_non_directory(format!("{}.json", record.request).as_ref())?;
                self.runtime_deadlines()?.remove(&record.request);
                removed += 1;
            }
        }
        if removed != 0 {
            self.root.sync()?;
        }
        Ok(removed)
    }
}

#[cfg(test)]
mod tests;
