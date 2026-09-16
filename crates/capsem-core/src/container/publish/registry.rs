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
    pub id: String,
    pub host_port: Option<u16>,
    pub guest_port: u16,
    pub target: PublicationTarget,
    pub access: capsem_proto::PublicationAccess,
    pub handle: P,
}

/// Declared publications keyed by their exposure identity. Loopback TCP keeps
/// its host-port text identity; browser previews use owner-generated UUIDs.
pub struct Registry<P> {
    entries: Mutex<BTreeMap<String, Declared<P>>>,
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
        entries.insert(declared.id.clone(), declared);
    }

    /// Forget a publication; the returned handle closes when dropped.
    pub fn remove(&self, id: &str) -> Option<Declared<P>> {
        self.entries.lock().unwrap().remove(id)
    }

    /// Live publications in stable exposure-id order.
    pub fn list<T>(&self, view: impl Fn(&Declared<P>) -> T) -> Vec<T> {
        let mut entries = self.entries.lock().unwrap();
        entries.retain(|_, entry| !entry.handle.is_finished());
        entries.values().map(view).collect()
    }

    pub fn with<T>(&self, id: &str, view: impl FnOnce(&Declared<P>) -> T) -> Option<T> {
        let mut entries = self.entries.lock().unwrap();
        entries.retain(|_, entry| !entry.handle.is_finished());
        entries.get(id).map(view)
    }

    pub fn find_map<T>(&self, mut view: impl FnMut(&Declared<P>) -> Option<T>) -> Option<T> {
        let mut entries = self.entries.lock().unwrap();
        entries.retain(|_, entry| !entry.handle.is_finished());
        entries.values().find_map(&mut view)
    }
}

#[cfg(test)]
mod tests;
