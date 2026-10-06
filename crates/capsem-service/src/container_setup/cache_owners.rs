//! Cache associations are observations of original workload/VM ownership.
use super::*;
use capsem_assets::oci::CacheKey;
use capsem_core::managed_sessions::VmBinding;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CacheOwner {
    pub(super) key: CacheKey,
    pub(super) vm: VmBinding,
}

impl ContainerSetups {
    pub(super) fn pin_image(
        &self,
        id: &str,
        generation: u64,
        manifest: Option<String>,
        owner: Option<CacheOwner>,
    ) -> bool {
        if owner.as_ref().is_some_and(|owner| owner.vm.id() != id) || (owner.is_some() && manifest.is_none()) {
            return false;
        }
        let mut records = self.records.lock().unwrap();
        let pinned = match records.get_mut(id) {
            Some(record) if record.generation == generation => {
                if record.manifest.is_some() && (record.manifest != manifest || record.cache_owner != owner) {
                    false
                } else {
                    record.manifest = manifest;
                    record.cache_owner = owner;
                    true
                }
            }
            _ => false,
        };
        drop(records);
        pinned
    }

    pub(super) fn active_cache_owners(&self, state: &ServiceState) -> Vec<CacheOwner> {
        let records = self.records.lock().unwrap();
        let mut active = records
            .values()
            .filter_map(|record| record.cache_owner.clone())
            .collect::<Vec<_>>();
        drop(records);
        let instances = state.instances.lock().unwrap();
        active.retain(|owner| {
            instances
                .get(owner.vm.id())
                .is_some_and(|instance| instance.generation == owner.vm.generation())
        });
        drop(instances);
        active.sort_unstable_by(|left, right| left.vm.id().cmp(right.vm.id()));
        active
    }

    pub(super) fn launch_record(&self, id: &str) -> Option<LaunchRecord> {
        let records = self.records.lock().unwrap();
        let record = records.get(id)?;
        let snapshot = LaunchRecord {
            image: record.status.image.clone(),
            digest: record.status.digest.clone().unwrap_or_default(),
            surface: record.status.surface.clone(),
            resolved: record.status.resolved.clone(),
            manifest: record.manifest.clone(),
            cache_key: record.cache_owner.as_ref().map(|owner| owner.key.as_str().to_owned()),
        };
        drop(records);
        Some(snapshot)
    }
}

#[cfg(test)]
mod tests;
