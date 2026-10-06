//! Cache-owned observations never grant execution or removal authority.

use std::collections::HashMap;

use anyhow::{Context, Result};
use capsem_foundation::unix::change_watch::ChangeWatch;

use super::CacheKey;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheState {
    Unknown,
    Missing,
    Partial,
    Ready,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CacheSnapshot {
    pub state: CacheState,
    pub verification_pending: bool,
    pub epoch: u64,
    pub verified_at_unix_ns: Option<u64>,
}

struct Observation {
    epoch: u64,
    state: CacheState,
    verified_at: Option<u64>,
    watch: Option<ChangeWatch>,
}

#[derive(Default)]
pub(super) struct Tracking {
    epoch: u64,
    entries: HashMap<CacheKey, Observation>,
}

impl Tracking {
    pub(super) fn epoch(&self) -> u64 {
        self.epoch
    }

    pub(super) fn invalidate(&mut self) -> Result<()> {
        self.entries.clear();
        self.epoch = self.epoch.checked_add(1).context("OCI mutation epoch exhausted")?;
        Ok(())
    }

    pub(super) fn invalidate_key(&mut self, key: &CacheKey) -> Result<()> {
        self.entries.remove(key);
        self.epoch = self.epoch.checked_add(1).context("OCI mutation epoch exhausted")?;
        Ok(())
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
        Ok(CacheSnapshot {
            state,
            verification_pending: state == CacheState::Unknown,
            epoch: observation.map_or(self.epoch, |entry| entry.epoch),
            verified_at_unix_ns: observation.and_then(|entry| entry.verified_at),
        })
    }

    pub(super) fn observed(
        &mut self,
        key: CacheKey,
        state: CacheState,
        verified_at: Option<u64>,
        watch: Option<ChangeWatch>,
    ) {
        self.entries.insert(
            key,
            Observation {
                epoch: self.epoch,
                state,
                verified_at,
                watch,
            },
        );
    }
}
