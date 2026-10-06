//! Local verification is owned work, separate from cheap cache observations.

use super::{CacheKey, ImageCache, Puller, RegistryAuth};
use anyhow::{ensure, Context, Result};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio::sync::watch;

#[derive(Clone)]
pub struct CacheReconciler(Arc<Owner>);

struct Owner {
    cache: ImageCache,
    maximum_keys: usize,
    state: Arc<Mutex<Work>>,
    jobs: watch::Sender<Option<Job>>,
    task: tokio::task::AbortHandle,
}

#[derive(Default)]
struct Work {
    generation: u64,
    active: bool,
    keys: Vec<CacheKey>,
}

#[derive(Clone)]
struct Job {
    generation: u64,
    keys: Vec<CacheKey>,
}

impl ImageCache {
    /// One worker and one bounded replacement batch. The caller supplies its
    /// explicit capacity and private staging directory; no runtime defaults
    /// or registry credentials become worker authority.
    pub fn reconciler(&self, architecture: &str, parent: PathBuf, maximum_keys: usize) -> Result<CacheReconciler> {
        ensure!(maximum_keys > 0, "cache worker capacity must be positive");
        super::pull::validate_architecture(architecture)?;
        let runtime = tokio::runtime::Handle::try_current().context("cache worker requires an async runtime")?;
        let state = Arc::new(Mutex::new(Work::default()));
        let (jobs, receiver) = watch::channel(None);
        let cache = self.clone();
        let work = Arc::clone(&state);
        let architecture = architecture.to_owned();
        let task = runtime.spawn(async move {
            let source = cache.clone();
            let puller = tokio::task::spawn_blocking(move || {
                Puller::new(&architecture, RegistryAuth::Anonymous).map(|puller| puller.with_cache(&source))
            })
            .await;
            match puller {
                Ok(Ok(puller)) => run(cache, puller, parent, work, receiver).await,
                _ => tracing::warn!("local cache verification worker could not initialize"),
            }
        });
        Ok(CacheReconciler(Arc::new(Owner {
            cache: self.clone(),
            maximum_keys,
            state,
            jobs,
            task: task.abort_handle(),
        })))
    }
}

impl CacheReconciler {
    /// Nonblocking scheduling: memory and owner notification snapshots only.
    /// Returns whether work changed. Equal active batches coalesce, while
    /// quiet negative or ready observations need no new hashes.
    pub fn request(&self, keys: &[CacheKey]) -> Result<bool> {
        ensure!(!self.0.jobs.is_closed(), "cache reconciliation worker stopped");
        ensure!(keys.len() <= self.0.maximum_keys, "cache worker batch exceeds capacity");
        let mut keys = keys.to_vec();
        keys.sort_unstable_by(|left, right| left.as_str().cmp(right.as_str()));
        keys.dedup();
        let mut work = self
            .0
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("cache worker state poisoned"))?;
        if work.active && work.keys == keys {
            return Ok(false);
        }
        let mut needed = work.active;
        if !needed {
            for key in &keys {
                needed |= self.0.cache.snapshot(key)?.verification_pending;
            }
        }
        if !needed {
            work.keys = keys;
            return Ok(false);
        }
        work.generation = work
            .generation
            .checked_add(1)
            .context("cache worker generation exhausted")?;
        work.active = true;
        work.keys = keys.clone();
        self.0.jobs.send_replace(Some(Job {
            generation: work.generation,
            keys,
        }));
        drop(work);
        Ok(true)
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        // The task holds cache/work state, never this owner. Last-owner drop
        // therefore aborts even while a local cache lease is contended.
        self.task.abort();
    }
}

async fn run(
    cache: ImageCache,
    puller: Puller,
    parent: PathBuf,
    state: Arc<Mutex<Work>>,
    mut jobs: watch::Receiver<Option<Job>>,
) {
    'jobs: loop {
        let current = jobs.borrow_and_update().clone();
        let Some(job) = current else {
            if jobs.changed().await.is_err() {
                return;
            }
            continue;
        };
        for key in &job.keys {
            match jobs.has_changed() {
                Ok(true) => continue 'jobs,
                Ok(false) => {}
                Err(_) => return,
            }
            if !cache.snapshot(key).is_ok_and(|snapshot| snapshot.verification_pending) {
                continue;
            }
            tokio::select! {
                biased;
                changed = jobs.changed() => {
                    if changed.is_err() { return; }
                    continue 'jobs;
                },
                result = puller.reconcile_cache(key, &parent) => {
                    if let Err(error) = result {
                        tracing::warn!(cache_key = %key.as_str(), %error, "local cache verification remains incomplete");
                    }
                },
            }
        }
        {
            let Ok(mut work) = state.lock() else {
                return;
            };
            if work.generation == job.generation {
                work.active = false;
            }
        }
        if jobs.changed().await.is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests;
