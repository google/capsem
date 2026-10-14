//! Generation-specific completion survives removal from the instance map.

use anyhow::{ensure, Result};
use capsem_foundation::unix::process::{self, ProcessId, Signal, SignalOutcome};
use std::{collections::HashMap, sync::Mutex};
use tokio::sync::{mpsc, oneshot, watch};
use uuid::Uuid;

#[derive(Clone, Copy)]
enum Outcome {
    Pending,
    Complete,
    Interrupted,
}

#[derive(Default)]
pub(crate) struct Retirements {
    entries: Mutex<HashMap<(String, Uuid), Entry>>,
}

#[derive(Clone)]
struct Entry {
    retired: watch::Receiver<Outcome>,
    exited: watch::Receiver<Outcome>,
    signals: mpsc::Sender<SignalRequest>,
}

struct SignalRequest {
    signal: Signal,
    reply: oneshot::Sender<std::io::Result<SignalOutcome>>,
}

impl Entry {
    fn exited(&self) -> Result<Option<SignalOutcome>> {
        match *self.exited.borrow() {
            Outcome::Pending => Ok(None),
            Outcome::Complete => Ok(Some(SignalOutcome::Gone)),
            Outcome::Interrupted => anyhow::bail!("original child wait did not complete"),
        }
    }
}

pub(crate) struct Registration {
    generation: Uuid,
    result: watch::Sender<Outcome>,
    exited: watch::Sender<Outcome>,
    signals: mpsc::Receiver<SignalRequest>,
    completed: bool,
}

impl Retirements {
    /// All fallible command preparation precedes this call. Register directly
    /// before spawn so a native spawn error is positive evidence of no child.
    pub(crate) fn spawn(
        &self,
        id: &str,
        generation: Uuid,
        command: &mut tokio::process::Command,
    ) -> Result<(tokio::process::Child, Registration)> {
        let registration = self.register(id, generation)?;
        match command.spawn() {
            Ok(child) => Ok((child, registration)),
            Err(error) => {
                registration.exited.send_replace(Outcome::Complete);
                registration.complete();
                Err(error.into())
            }
        }
    }

    pub(crate) fn register(&self, id: &str, generation: Uuid) -> Result<Registration> {
        ensure!(!generation.is_nil(), "retirement generation is nil");
        let mut entries = self.entries.lock().unwrap();
        let key = (id.to_owned(), generation);
        ensure!(!entries.contains_key(&key), "generation retirement already registered");
        let (result, receiver) = watch::channel(Outcome::Pending);
        let (exited, exit_receiver) = watch::channel(Outcome::Pending);
        let (signals, signal_receiver) = mpsc::channel(8);
        entries.insert(
            key,
            Entry {
                retired: receiver,
                exited: exit_receiver,
                signals,
            },
        );
        drop(entries);
        Ok(Registration {
            generation,
            result,
            exited,
            signals: signal_receiver,
            completed: false,
        })
    }

    /// Absence is not proof of completed cleanup. Completion remains
    /// idempotently observable for subsequent generation-bound reconciliation.
    pub(crate) async fn wait(&self, id: &str, generation: Uuid) -> Result<bool> {
        let key = (id.to_owned(), generation);
        let Some(entry) = self.entries.lock().unwrap().get(&key).cloned() else {
            return Ok(false);
        };
        wait(entry.retired).await
    }

    pub(crate) async fn wait_exit(&self, id: &str, generation: Uuid) -> Result<bool> {
        let entry = self.entry(id, generation)?;
        wait(entry.exited).await
    }

    pub(crate) fn is_live(&self, id: &str, generation: Uuid) -> Result<bool> {
        Ok(self.entry(id, generation)?.exited()?.is_none())
    }

    fn entry(&self, id: &str, generation: Uuid) -> Result<Entry> {
        self.entries
            .lock()
            .unwrap()
            .get(&(id.to_owned(), generation))
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("original child owner is unavailable"))
    }

    /// Commands go to the sole owner of the unreaped child, never a PID
    /// supplied by an instance-map or process-table observation.
    pub(crate) async fn signal(&self, id: &str, generation: Uuid, signal: Signal) -> Result<SignalOutcome> {
        let entry = self.entry(id, generation)?;
        if let Some(outcome) = entry.exited()? {
            return Ok(outcome);
        }
        let (reply, result) = oneshot::channel();
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            if entry.signals.send(SignalRequest { signal, reply }).await.is_err() {
                return entry
                    .exited()?
                    .ok_or_else(|| anyhow::anyhow!("original child signal owner interrupted"));
            }
            match result.await {
                Ok(result) => Ok(result?),
                Err(_) => entry
                    .exited()?
                    .ok_or_else(|| anyhow::anyhow!("original child signal owner interrupted")),
            }
        })
        .await
        .map_err(|_| anyhow::anyhow!("original child signal owner timed out"))?
    }
}

async fn wait(mut result: watch::Receiver<Outcome>) -> Result<bool> {
    loop {
        let outcome = *result.borrow_and_update();
        match outcome {
            Outcome::Pending => result.changed().await?,
            Outcome::Complete => return Ok(true),
            Outcome::Interrupted => anyhow::bail!("original child reaper did not complete"),
        }
    }
}

impl Registration {
    pub(crate) fn generation(&self) -> Uuid {
        self.generation
    }

    pub(crate) async fn wait_child(
        &mut self,
        child: &mut tokio::process::Child,
    ) -> std::io::Result<std::process::ExitStatus> {
        self.wait_child_before_reap(child, || ())
            .await
            .map(|(status, ())| status)
    }

    pub(crate) async fn wait_child_before_reap<T>(
        &mut self,
        child: &mut tokio::process::Child,
        before_reap: impl FnOnce() -> T,
    ) -> std::io::Result<(std::process::ExitStatus, T)> {
        let pid = child
            .id()
            .ok_or_else(|| std::io::Error::other("original child identity unavailable"))?;
        let pid = ProcessId::try_from(pid)?;
        let mut observed = tokio::task::spawn_blocking(move || process::wait_for_child_exit(pid));
        let observation = loop {
            tokio::select! {
                // Retain the original PID until generation authority is revoked,
                // while continuing to service owned shutdown signals.
                biased;
                result = &mut observed => break result.map_err(std::io::Error::other).and_then(|result| result),
                Some(request) = self.signals.recv() => {
                    let outcome = child.id().ok_or_else(|| std::io::Error::other("original child identity unavailable"))
                        .and_then(ProcessId::try_from)
                        .and_then(|pid| process::send_signal(pid,request.signal));
                    let _ = request.reply.send(outcome);
                }
            }
        };
        observation?;
        let revoked = before_reap();
        let status = child.wait().await;
        self.exited.send_replace(if status.is_ok() {
            Outcome::Complete
        } else {
            Outcome::Interrupted
        });
        self.signals.close();
        while let Ok(request) = self.signals.try_recv() {
            let outcome = if status.is_ok() {
                Ok(SignalOutcome::Gone)
            } else {
                Err(std::io::Error::other("original child wait failed"))
            };
            let _ = request.reply.send(outcome);
        }
        status.map(|status| (status, revoked))
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
        if matches!(*self.exited.borrow(), Outcome::Pending) {
            self.exited.send_replace(Outcome::Interrupted);
        }
    }
}

#[cfg(test)]
mod tests;
