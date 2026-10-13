use std::hash::Hash;
use std::marker::PhantomData;
use std::sync::Arc;
use std::time::Instant;

use quick_cache::sync::{Cache, GuardResult};
use quick_cache::{DefaultHashBuilder, Equivalent, Lifecycle, UnitWeighter};

use crate::telemetry::{CacheMetrics, CacheStats, LoadStatus};

struct EvictionLifecycle<K, V> {
    telemetry: CacheMetrics,
    marker: PhantomData<fn(K, V)>,
}

struct EvictionState {
    count: bool,
}

impl Default for EvictionState {
    fn default() -> Self {
        Self { count: true }
    }
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
    type RequestState = EvictionState;

    fn on_evict(&self, state: &mut Self::RequestState, _key: K, _value: Arc<V>) {
        if state.count {
            self.telemetry.evict();
        }
    }
}

type Inner<K, V> = Cache<K, Arc<V>, UnitWeighter, DefaultHashBuilder, EvictionLifecycle<K, V>>;

pub struct BoundedCache<K, V> {
    inner: Inner<K, V>,
    telemetry: CacheMetrics,
}

impl<K, V> BoundedCache<K, V>
where
    K: Eq + Hash,
{
    pub fn new(name: &'static str, capacity: usize) -> Self {
        let capacity = capacity.max(1);
        let telemetry = CacheMetrics::new(name);
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
        self.get_if(key, |_| true)
    }

    pub fn get_if<Q>(&self, key: &Q, accept: impl FnOnce(&V) -> bool) -> Option<Arc<V>>
    where
        Q: Equivalent<K> + Hash + ?Sized,
    {
        let value = self.inner.get(key).filter(|value| accept(value));
        if value.is_some() {
            self.telemetry.hit();
        } else {
            self.telemetry.miss();
        }
        value
    }

    pub(crate) fn peek<Q>(&self, key: &Q) -> Option<Arc<V>>
    where
        Q: Equivalent<K> + Hash + ?Sized,
    {
        self.inner.get(key)
    }

    pub fn insert(&self, key: K, value: V) -> Arc<V> {
        let value = Arc::new(value);
        if self.inner.peek(&key).is_some() {
            let mut lifecycle = EvictionState { count: false };
            match self
                .inner
                .replace_with_lifecycle(key, Arc::clone(&value), false, &mut lifecycle)
            {
                Ok(()) => self.telemetry.refresh(),
                Err((key, value)) => {
                    self.inner.insert(key, value);
                    self.telemetry.insertion();
                }
            }
        } else {
            self.inner.insert(key, Arc::clone(&value));
            self.telemetry.insertion();
        }
        self.telemetry.set_entries(self.inner.len());
        value
    }

    pub fn get_or_try_insert_with<Q, E>(&self, key: &Q, load: impl FnOnce() -> Result<V, E>) -> Result<Arc<V>, E>
    where
        Q: Equivalent<K> + Hash + ToOwned<Owned = K> + ?Sized,
    {
        // The guarded lookup coordinates misses, but its waiter bookkeeping is
        // unnecessary on the overwhelmingly common resident path. Keep hits
        // on quick_cache's concurrent read path and acquire a load guard only
        // after an observed miss. A racing insert is still returned by the
        // guarded lookup below, preserving exact single-flight behavior.
        if let Some(value) = self.inner.get(key) {
            self.telemetry.hit();
            return Ok(value);
        }
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
