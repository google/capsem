//! One lifetime private address per VM, handed out from the host's pool.
//!
//! The allocator is the in-memory authority; durability is the persistent
//! VM registry, which records each named VM's address and reserves it here
//! again at startup. Ephemeral VMs hold theirs only while they run.
//!
//! Allocation walks a cursor around the pool rather than taking the lowest
//! free address, so an address freed by a deleted VM is not the next one
//! handed out: audit rows that name it keep pointing at one VM for as long
//! as the pool allows.

use capsem_config::PrivatePool;
use std::collections::BTreeSet;
use std::net::Ipv4Addr;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AddressError {
    #[error("private pool {pool} is exhausted: all {capacity} addresses are in use")]
    Exhausted { pool: PrivatePool, capacity: u32 },
    #[error("{address} is outside the private pool {pool}")]
    OutsidePool { address: Ipv4Addr, pool: PrivatePool },
    #[error("{address} is the pool's gateway, network or broadcast address")]
    Reserved { address: Ipv4Addr },
    #[error("{address} is already assigned to another VM")]
    InUse { address: Ipv4Addr },
}

pub struct AddressAllocator {
    pool: PrivatePool,
    in_use: BTreeSet<Ipv4Addr>,
    /// The next candidate, as an offset from the first host.
    cursor: u32,
}

impl AddressAllocator {
    pub fn new(pool: PrivatePool) -> Self {
        Self {
            pool,
            in_use: BTreeSet::new(),
            cursor: 0,
        }
    }

    pub fn pool(&self) -> PrivatePool {
        self.pool
    }

    pub fn in_use(&self) -> usize {
        self.in_use.len()
    }

    /// Claim an address recorded elsewhere -- a persistent VM's at startup.
    pub fn reserve(&mut self, address: Ipv4Addr) -> Result<(), AddressError> {
        if !self.pool.contains(address) {
            return Err(AddressError::OutsidePool {
                address,
                pool: self.pool,
            });
        }
        if address < self.pool.first_host() || address > self.pool.last_host() {
            return Err(AddressError::Reserved { address });
        }
        if !self.in_use.insert(address) {
            return Err(AddressError::InUse { address });
        }
        Ok(())
    }

    /// The next free address after the cursor, wrapping once.
    pub fn allocate(&mut self) -> Result<Ipv4Addr, AddressError> {
        let capacity = self.pool.capacity();
        let base = self.pool.first_host().to_bits();
        for step in 0..capacity {
            let offset = (self.cursor + step) % capacity;
            let candidate = Ipv4Addr::from_bits(base + offset);
            if self.in_use.insert(candidate) {
                self.cursor = (offset + 1) % capacity;
                return Ok(candidate);
            }
        }
        Err(AddressError::Exhausted {
            pool: self.pool,
            capacity,
        })
    }

    /// Give an address back. `false` when it was not held, which a caller
    /// releasing on a cleanup path treats as already done.
    pub fn release(&mut self, address: Ipv4Addr) -> bool {
        self.in_use.remove(&address)
    }
}

#[cfg(test)]
mod tests;
