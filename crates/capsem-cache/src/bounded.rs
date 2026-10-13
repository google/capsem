use std::hash::Hash;
use std::marker::PhantomData;
use std::sync::Arc;
use std::time::Instant;

use quick_cache::sync::{Cache, GuardResult};
use quick_cache::{DefaultHashBuilder, Equivalent, Lifecycle, UnitWeighter};

use crate::telemetry::{CacheStats, CacheTelemetry, LoadStatus};

struct EvictionLifecycle<K, V> {
    telemetry: CacheTelemetry,
    marker: PhantomData<fn(K, V)>,
}

impl<K, V> Clone for EvictionLifecycle<K, V> {
    fn clone(&self) -> Self {
        Self {
            telemetry: self.telemetry.clone(),
            marker: PhantomData,
        }
    }
}

impl<K, V> Lifecycle<K, Arc<V>> for EvictionLifecycle<K, V> {
    type RequestState = ();

    fn on_evict(&self, _state: &mut Self::RequestState, _key: K, _value: Arc<V>) {
        self.telemetry.evict();
    }
}

type Inner<K, V> = Cache<K, Arc<V>, UnitWeighter, DefaultHashBuilder, EvictionLifecycle<K, V>>;

pub struct BoundedCache<K, V> {
    inner: Inner<K, V>,
    telemetry: CacheTelemetry,
}

impl<K, V> BoundedCache<K, V>
where
    K: Eq + Hash,
{
    pub fn new(name: &'static str, capacity: usize) -> Self {
        let capacity = capacity.max(1);
        let telemetry = CacheTelemetry::new(name);
        Self {
            inner: Cache::with(
                capacity,
                u64::try_from(capacity).unwrap_or(u64::MAX),
                UnitWeighter,
                DefaultHashBuilder::default(),
                EvictionLifecycle {
                    telemetry: telemetry.clone(),
                    marker: PhantomData,
                },
            ),
            telemetry,
        }
    }

    pub fn get<Q>(&self, key: &Q) -> Option<Arc<V>>
    where
        Q: Equivalent<K> + Hash + ?Sized,
    {
        let value = self.inner.get(key);
        if value.is_some() {
            self.telemetry.hit();
        } else {
            self.telemetry.miss();
        }
        value
    }

    pub fn insert(&self, key: K, value: V) -> Arc<V> {
        let replacing = self.inner.peek(&key).is_some();
        let value = Arc::new(value);
        self.inner.insert(key, Arc::clone(&value));
        if replacing {
            self.telemetry.refresh();
        } else {
            self.telemetry.insertion();
        }
        self.telemetry.set_entries(self.inner.len());
        value
    }

    pub fn get_or_try_insert_with<Q, E>(&self, key: &Q, load: impl FnOnce() -> Result<V, E>) -> Result<Arc<V>, E>
    where
        Q: Equivalent<K> + Hash + ToOwned<Owned = K> + ?Sized,
    {
        match self.inner.get_value_or_guard(key, None) {
            GuardResult::Value(value) => {
                self.telemetry.hit();
                Ok(value)
            }
            GuardResult::Guard(guard) => {
                self.telemetry.miss();
                let started = Instant::now();
                match load() {
                    Ok(value) => {
                        let value = Arc::new(value);
                        let _inserted = guard.insert(Arc::clone(&value));
                        self.telemetry.load(started.elapsed(), LoadStatus::Success);
                        self.telemetry.insertion();
                        self.telemetry.set_entries(self.inner.len());
                        Ok(value)
                    }
                    Err(error) => {
                        self.telemetry.load(started.elapsed(), LoadStatus::Error);
                        Err(error)
                    }
                }
            }
            GuardResult::Timeout => unreachable!("an unbounded cache wait cannot time out"),
        }
    }

    pub fn remove<Q>(&self, key: &Q) -> Option<Arc<V>>
    where
        Q: Equivalent<K> + Hash + ?Sized,
    {
        let removed = self.inner.remove(key).map(|(_, value)| value);
        if removed.is_some() {
            self.telemetry.invalidate(1);
            self.telemetry.set_entries(self.inner.len());
        }
        removed
    }

    pub fn retain(&self, retain: impl Fn(&K, &Arc<V>) -> bool) {
        let before = self.inner.len();
        self.inner.retain(retain);
        let after = self.inner.len();
        self.telemetry
            .invalidate(u64::try_from(before.saturating_sub(after)).unwrap_or(u64::MAX));
        self.telemetry.set_entries(after);
    }

    pub fn clear(&self) {
        let before = self.inner.len();
        self.inner.clear();
        self.telemetry.invalidate(u64::try_from(before).unwrap_or(u64::MAX));
        self.telemetry.set_entries(0);
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn stats(&self) -> CacheStats {
        self.telemetry.snapshot()
    }
}
