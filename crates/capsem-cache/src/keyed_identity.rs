use std::hash::Hash;
use std::sync::Arc;

use quick_cache::Equivalent;

use crate::{BoundedCache, CacheStats};

struct IdentityEntry<I, V> {
    identity: I,
    value: Arc<V>,
}

/// A bounded cache whose values are fresh only for the caller's identity.
pub struct KeyedIdentityCache<K, I, V> {
    entries: BoundedCache<K, IdentityEntry<I, V>>,
}

impl<K, I, V> KeyedIdentityCache<K, I, V>
where
    K: Eq + Hash,
    I: Eq,
{
    pub fn new(name: &'static str, capacity: usize) -> Self {
        Self {
            entries: BoundedCache::new(name, capacity),
        }
    }

    pub fn get<Q>(&self, key: &Q, identity: &I) -> Option<Arc<V>>
    where
        Q: Equivalent<K> + Hash + ?Sized,
    {
        self.entries
            .get_if(key, |entry| &entry.identity == identity)
            .map(|entry| Arc::clone(&entry.value))
    }

    pub fn publish(&self, key: K, identity: I, value: V) -> Arc<V> {
        if let Some(entry) = self.entries.peek(&key).filter(|entry| entry.identity == identity) {
            return Arc::clone(&entry.value);
        }
        let value = Arc::new(value);
        self.entries.insert(
            key,
            IdentityEntry {
                identity,
                value: Arc::clone(&value),
            },
        );
        value
    }

    pub fn remove<Q>(&self, key: &Q) -> Option<Arc<V>>
    where
        Q: Equivalent<K> + Hash + ?Sized,
    {
        self.entries.remove(key).map(|entry| Arc::clone(&entry.value))
    }

    pub fn retain(&self, retain: impl Fn(&K) -> bool) {
        self.entries.retain(|key, _| retain(key));
    }

    pub fn clear(&self) {
        self.entries.clear();
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn stats(&self) -> CacheStats {
        self.entries.stats()
    }
}
