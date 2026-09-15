//! The VM owner's declared publications: the one registry the service asks.

use capsem_proto::PublicationTarget;
use std::collections::BTreeMap;
use std::sync::Mutex;

/// A handle whose work may end on its own (a broker that stopped).
pub trait Live {
    fn is_finished(&self) -> bool;
}

impl Live for super::Publication {
    fn is_finished(&self) -> bool {
        super::Publication::is_finished(self)
    }
}

/// One declared listener and the handle keeping it open.
pub struct Declared<P> {
    pub host_port: u16,
    pub guest_port: u16,
    pub target: PublicationTarget,
    pub handle: P,
}

/// Declared publications keyed by their loopback host port, which is unique
/// on the host and is the exposure's identity.
pub struct Registry<P> {
    entries: Mutex<BTreeMap<u16, Declared<P>>>,
}

impl<P> Default for Registry<P> {
    fn default() -> Self {
        Self {
            entries: Mutex::new(BTreeMap::new()),
        }
    }
}

impl<P: Live> Registry<P> {
    /// Record a publication. Dropping a replaced handle releases its listener.
    pub fn insert(&self, declared: Declared<P>) {
        let mut entries = self.entries.lock().unwrap();
        entries.retain(|_, entry| !entry.handle.is_finished());
        entries.insert(declared.host_port, declared);
    }

    /// Forget a publication; the returned handle closes when dropped.
    pub fn remove(&self, host_port: u16) -> Option<Declared<P>> {
        self.entries.lock().unwrap().remove(&host_port)
    }

    /// Live publications in host port order, as `(host, guest, target, handle view)`.
    pub fn list<T>(&self, view: impl Fn(&Declared<P>) -> T) -> Vec<T> {
        let mut entries = self.entries.lock().unwrap();
        entries.retain(|_, entry| !entry.handle.is_finished());
        entries.values().map(view).collect()
    }
}

#[cfg(test)]
mod tests;
