//! Cache-owned observations never grant execution or removal authority.

use std::{collections::HashMap, sync::Arc};

use anyhow::{Context, Result};
use capsem_foundation::unix::change_watch::ChangeWatch;

use super::{CacheInventory, CacheKey};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheState {
    Unknown,
    Missing,
    Partial,
    Ready,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheReason {
    UnsupportedPlatform,
    IncompatibleRuntime,
    ReceiptMissing,
    BlobMissing,
    IntegrityInvalid,
    VerificationPending,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CacheSnapshot {
    pub state: CacheState,
    pub reason: Option<CacheReason>,
    pub verification_pending: bool,
    pub epoch: u64,
    pub verified_at_unix_ns: Option<u64>,
}

struct Observation {
    epoch: u64,
    state: CacheState,
    reason: Option<CacheReason>,
    verified_at: Option<u64>,
    watch: Option<ChangeWatch>,
}

struct InventoryObservation {
    value: Arc<CacheInventory>,
    watch: ChangeWatch,
}

#[derive(Default)]
pub(super) struct Tracking {
    epoch: u64,
    entries: HashMap<CacheKey, Observation>,
    inventory: Option<InventoryObservation>,
}

impl Tracking {
    pub(super) fn epoch(&self) -> u64 {
        self.epoch
    }

    pub(super) fn invalidate(&mut self) -> Result<()> {
        self.entries.clear();
        self.invalidate_inventory()
    }

    pub(super) fn invalidate_key(&mut self, key: &CacheKey) -> Result<()> {
        self.entries.remove(key);
        self.invalidate_inventory()
    }

    pub(super) fn invalidate_inventory(&mut self) -> Result<()> {
        self.inventory = None;
        self.epoch = self.epoch.checked_add(1).context("OCI mutation epoch exhausted")?;
        Ok(())
    }

    pub(super) fn inventory_snapshot(&mut self) -> Result<Option<Arc<CacheInventory>>> {
        if let Some(observation) = &mut self.inventory {
            match observation.watch.changed() {
                Ok(false) => {}
                result => {
                    self.invalidate_inventory()?;
                    result?;
                }
            }
        }
        Ok(self
            .inventory
            .as_ref()
            .map(|observation| Arc::clone(&observation.value)))
    }

    pub(super) fn observed_inventory(&mut self, value: CacheInventory, watch: ChangeWatch) {
        self.inventory = Some(InventoryObservation {
            value: Arc::new(value),
            watch,
        });
    }

    pub(super) fn snapshot(&mut self, key: &CacheKey) -> Result<CacheSnapshot> {
        if let Some(watch) = self.entries.get_mut(key).and_then(|entry| entry.watch.as_mut()) {
            match watch.changed() {
                Ok(false) => {}
                result => {
                    self.invalidate()?;
                    result?;
                }
            }
        }
        let observation = self.entries.get(key);
        let state = observation.map_or(CacheState::Unknown, |entry| entry.state);
        let reason = observation.map_or(Some(CacheReason::VerificationPending), |entry| entry.reason);
        Ok(CacheSnapshot {
            state,
            reason,
            verification_pending: reason == Some(CacheReason::VerificationPending),
            epoch: observation.map_or(self.epoch, |entry| entry.epoch),
            verified_at_unix_ns: observation.and_then(|entry| entry.verified_at),
        })
    }

    pub(super) fn observed(
        &mut self,
        key: CacheKey,
        state: CacheState,
        reason: Option<CacheReason>,
        verified_at: Option<u64>,
        watch: Option<ChangeWatch>,
    ) {
        self.entries.insert(
            key,
            Observation {
                epoch: self.epoch,
                state,
                reason,
                verified_at,
                watch,
            },
        );
    }
}
