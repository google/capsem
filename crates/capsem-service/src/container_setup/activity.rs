//! Retirement counts include VM-targeted blocking jobs whose async waiter
//! can be aborted while the underlying filesystem operation continues.

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex, Weak,
    },
};
use tokio::sync::{watch, Notify};

pub(super) type Retiring = Arc<Mutex<HashMap<String, Vec<Activity>>>>;

#[derive(Clone)]
pub(super) struct Activity(Arc<State>);

struct State {
    id: String,
    generation: u64,
    running: AtomicUsize,
    idle: Notify,
    cancel: watch::Sender<bool>,
    retiring: Weak<Mutex<HashMap<String, Vec<Activity>>>>,
}

pub(super) struct Lease(Arc<State>);

impl Activity {
    pub(super) fn new(id: &str, generation: u64, retiring: &Retiring) -> Self {
        Self(Arc::new(State {
            id: id.into(),
            generation,
            running: AtomicUsize::new(0),
            idle: Notify::new(),
            cancel: watch::channel(false).0,
            retiring: Arc::downgrade(retiring),
        }))
    }
    pub(super) fn enter(&self) -> Lease {
        self.0.running.fetch_add(1, Ordering::AcqRel);
        Lease(Arc::clone(&self.0))
    }
    pub(super) fn retire(&self) {
        self.0.cancel.send_replace(true);
        if let Some(retiring) = self.0.retiring.upgrade() {
            let mut retiring = retiring.lock().unwrap();
            if self.0.running.load(Ordering::Acquire) != 0 {
                retiring.entry(self.0.id.clone()).or_default().push(self.clone());
            }
        }
    }
    pub(super) async fn wait(&self) {
        loop {
            let notified = self.0.idle.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.0.running.load(Ordering::Acquire) == 0 {
                return;
            }
            notified.await;
        }
    }
}

impl Lease {
    pub(super) async fn cancelled(&self) {
        let mut cancelled = self.0.cancel.subscribe();
        while !*cancelled.borrow_and_update() {
            if cancelled.changed().await.is_err() {
                return;
            }
        }
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        if self.0.running.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.0.idle.notify_waiters();
            if let Some(retiring) = self.0.retiring.upgrade() {
                let mut retiring = retiring.lock().unwrap();
                if let Some(generations) = retiring.get_mut(&self.0.id) {
                    generations.retain(|activity| activity.0.generation != self.0.generation);
                    if generations.is_empty() {
                        retiring.remove(&self.0.id);
                    }
                }
            }
        }
    }
}
