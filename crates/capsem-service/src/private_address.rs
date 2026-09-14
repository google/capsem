//! One lifetime private address per VM: leased at create, held by the
//! running instance or the registry entry, released when neither holds it.
use super::{InstanceInfo, PersistentRegistry, ServiceState};
use anyhow::Result;
use tracing::warn;

/// An address allocated for a VM that is not yet recorded anywhere. Dropped
/// without `commit`, it goes back to the pool, so every early return between
/// allocation and the instance or registry record releases it.
pub(crate) struct AddressLease<'a> {
    state: &'a ServiceState,
    pub(crate) address: std::net::Ipv4Addr,
    committed: bool,
}

impl AddressLease<'_> {
    pub(crate) fn commit(mut self) -> std::net::Ipv4Addr {
        self.committed = true;
        self.address
    }
}

impl Drop for AddressLease<'_> {
    fn drop(&mut self) {
        if !self.committed {
            self.state.private_addresses.lock().unwrap().release(self.address);
        }
    }
}

impl ServiceState {
    pub(crate) fn lease_private_address(&self) -> Result<AddressLease<'_>> {
        let address = self.private_addresses.lock().unwrap().allocate()?;
        Ok(AddressLease {
            state: self,
            address,
            committed: false,
        })
    }

    /// Remove a running instance and, for an ephemeral VM, return its
    /// address to the pool. A persistent VM keeps its address while its
    /// registry entry exists.
    pub(crate) fn evict_instance(&self, id: &str) -> Option<InstanceInfo> {
        let removed = self.instances.lock().unwrap().remove(id);
        if let Some(info) = &removed {
            self.release_if_ephemeral(info);
        }
        removed
    }

    pub(crate) fn release_if_ephemeral(&self, info: &InstanceInfo) {
        if !info.persistent {
            self.private_addresses.lock().unwrap().release(info.private_address);
        }
    }

    /// Unregister a persistent VM and return its address to the pool.
    /// Absent entries are already forgotten.
    pub(crate) fn forget_persistent_entry(&self, name: &str) -> Result<()> {
        let address = {
            let registry = self.persistent_registry.lock().unwrap();
            let Some(address) = registry.get(name).map(|entry| entry.private_address) else {
                return Ok(());
            };
            registry.unregister(name)?;
            address
        };
        if let Some(address) = address {
            self.private_addresses.lock().unwrap().release(address);
        }
        Ok(())
    }
}

/// Reserve every persistent VM's recorded address at startup. An address the
/// pool refuses -- outside it, or already taken by another entry -- is
/// dropped from that entry with a warning, so resume assigns a fresh one
/// instead of two VMs sharing a link address.
pub(crate) fn reserve_registry_addresses(
    registry: &mut PersistentRegistry,
    allocator: &mut capsem_core::net::address_pool::AddressAllocator,
) {
    // Oldest entry first, then by name: which VM keeps a contested address
    // must not depend on hash order between two service starts.
    let mut names: Vec<(String, String)> = registry
        .data
        .vms
        .values()
        .map(|entry| (entry.created_at.clone(), entry.name.clone()))
        .collect();
    names.sort();
    for (_, name) in names {
        let Some(entry) = registry.data.vms.get_mut(&name) else {
            continue;
        };
        let Some(address) = entry.private_address else {
            continue;
        };
        if let Err(error) = allocator.reserve(address) {
            warn!(vm = %entry.name, %address, %error, "dropping unusable private address from registry");
            entry.private_address = None;
        }
    }
}
