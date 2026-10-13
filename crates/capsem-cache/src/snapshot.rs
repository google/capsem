use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use crate::telemetry::{CacheStats, CacheTelemetry, LoadStatus};

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

struct IdentityEntry<I, V> {
    identity: I,
    value: Arc<V>,
}

pub struct IdentityCache<I, V> {
    entry: Mutex<Option<IdentityEntry<I, V>>>,
    telemetry: CacheTelemetry,
}

impl<I: Eq, V> IdentityCache<I, V> {
    pub fn new(name: &'static str) -> Self {
        Self {
            entry: Mutex::new(None),
            telemetry: CacheTelemetry::new(name),
        }
    }

    pub fn get(&self, identity: &I) -> Option<Arc<V>> {
        let value = lock(&self.entry)
            .as_ref()
            .filter(|entry| &entry.identity == identity)
            .map(|entry| Arc::clone(&entry.value));
        if value.is_some() {
            self.telemetry.hit();
        } else {
            self.telemetry.miss();
        }
        value
    }

    pub fn publish(&self, identity: I, value: V) -> Arc<V> {
        let mut entry = lock(&self.entry);
        if let Some(existing) = entry.as_ref().filter(|entry| entry.identity == identity) {
            return Arc::clone(&existing.value);
        }
        let refresh = entry.is_some();
        let value = Arc::new(value);
        *entry = Some(IdentityEntry {
            identity,
            value: Arc::clone(&value),
        });
        drop(entry);
        if refresh {
            self.telemetry.refresh();
        } else {
            self.telemetry.insertion();
        }
        self.telemetry.set_entries(1);
        value
    }

    pub fn invalidate(&self) {
        let removed = lock(&self.entry).take().is_some();
        if removed {
            self.telemetry.invalidate(1);
            self.telemetry.set_entries(0);
        }
    }

    pub fn record_load(&self, elapsed: Duration, status: LoadStatus) {
        self.telemetry.load(elapsed, status);
    }

    pub fn stats(&self) -> CacheStats {
        self.telemetry.snapshot()
    }
}

impl<I: Eq, V> Default for IdentityCache<I, V> {
    fn default() -> Self {
        Self::new("unnamed")
    }
}

struct GenerationState<V> {
    generation: u64,
    value: Option<Arc<V>>,
}

pub struct GenerationLookup<V> {
    generation: u64,
    value: Option<Arc<V>>,
}

impl<V> GenerationLookup<V> {
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn value(&self) -> Option<Arc<V>> {
        self.value.as_ref().map(Arc::clone)
    }
}

pub struct GenerationCache<V> {
    state: Mutex<GenerationState<V>>,
    telemetry: CacheTelemetry,
}

impl<V> GenerationCache<V> {
    pub fn new(name: &'static str) -> Self {
        Self {
            state: Mutex::new(GenerationState {
                generation: 0,
                value: None,
            }),
            telemetry: CacheTelemetry::new(name),
        }
    }

    pub fn lookup(&self) -> GenerationLookup<V> {
        let state = lock(&self.state);
        let lookup = GenerationLookup {
            generation: state.generation,
            value: state.value.as_ref().map(Arc::clone),
        };
        drop(state);
        if lookup.value.is_some() {
            self.telemetry.hit();
        } else {
            self.telemetry.miss();
        }
        lookup
    }

    pub fn publish(&self, generation: u64, value: V) -> Option<Arc<V>> {
        let mut state = lock(&self.state);
        if state.generation != generation {
            return None;
        }
        if let Some(existing) = &state.value {
            return Some(Arc::clone(existing));
        }
        let value = Arc::new(value);
        state.value = Some(Arc::clone(&value));
        drop(state);
        self.telemetry.insertion();
        self.telemetry.set_entries(1);
        Some(value)
    }

    pub fn invalidate(&self) {
        let mut state = lock(&self.state);
        state.generation = state.generation.wrapping_add(1);
        state.value = None;
        drop(state);
        self.telemetry.invalidate(1);
        self.telemetry.set_entries(0);
    }

    pub fn record_load(&self, elapsed: Duration, status: LoadStatus) {
        self.telemetry.load(elapsed, status);
    }

    pub fn stats(&self) -> CacheStats {
        self.telemetry.snapshot()
    }
}

impl<V> Default for GenerationCache<V> {
    fn default() -> Self {
        Self::new("unnamed")
    }
}
