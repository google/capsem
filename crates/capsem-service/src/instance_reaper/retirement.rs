//! Generation-specific completion survives removal from the instance map.

use anyhow::{ensure, Result};
use std::{collections::HashMap, sync::Mutex};
use tokio::sync::watch;
use uuid::Uuid;

#[derive(Clone, Copy)]
enum Outcome {
    Pending,
    Complete,
    Interrupted,
}

#[derive(Default)]
pub(crate) struct Retirements {
    entries: Mutex<HashMap<(String, Uuid), watch::Receiver<Outcome>>>,
}

pub(crate) struct Registration {
    generation: Uuid,
    result: watch::Sender<Outcome>,
    completed: bool,
}

impl Retirements {
    pub(crate) fn register(&self, id: &str, generation: Uuid) -> Result<Registration> {
        ensure!(!generation.is_nil(), "retirement generation is nil");
        let mut entries = self.entries.lock().unwrap();
        let key = (id.to_owned(), generation);
        ensure!(!entries.contains_key(&key), "generation retirement already registered");
        let (result, receiver) = watch::channel(Outcome::Pending);
        entries.insert(key, receiver);
        drop(entries);
        Ok(Registration {
            generation,
            result,
            completed: false,
        })
    }

    /// Absence is not proof of completed cleanup. Completion remains
    /// idempotently observable for subsequent generation-bound reconciliation.
    pub(crate) async fn wait(&self, id: &str, generation: Uuid) -> Result<bool> {
        let key = (id.to_owned(), generation);
        let Some(mut result) = self.entries.lock().unwrap().get(&key).cloned() else {
            return Ok(false);
        };
        loop {
            let outcome = *result.borrow_and_update();
            match outcome {
                Outcome::Pending => result.changed().await?,
                Outcome::Complete => return Ok(true),
                Outcome::Interrupted => anyhow::bail!("original child reaper did not complete"),
            }
        }
    }
}

impl Registration {
    pub(crate) fn generation(&self) -> Uuid {
        self.generation
    }
    pub(crate) fn complete(mut self) {
        self.completed = true;
        self.result.send_replace(Outcome::Complete);
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        if !self.completed {
            self.result.send_replace(Outcome::Interrupted);
        }
    }
}
