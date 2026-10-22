//! Bounded recording owner; terminal callers never perform storage I/O.

use super::{record_frame, PtyLog, DEFAULT_MAX_BYTES, DIR_INPUT, DIR_OUTPUT};
use std::io;
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, oneshot, watch, OwnedSemaphorePermit, Semaphore};

// At most one log generation is retained in memory, including the frame on
// disk's stack. A record-count cap also bounds tiny frames and flush requests.
const QUEUE_BYTES: usize = DEFAULT_MAX_BYTES as usize;
const QUEUE_RECORDS: usize = 256;

enum Command {
    Record {
        frame: Vec<u8>,
        _bytes: OwnedSemaphorePermit,
    },
    Flush(oneshot::Sender<io::Result<()>>),
}

#[derive(Clone)]
struct Failure {
    kind: io::ErrorKind,
    message: String,
}

impl Failure {
    fn error(&self) -> io::Error {
        io::Error::new(self.kind, self.message.clone())
    }
}

struct Admission {
    sender: Option<mpsc::Sender<Command>>,
    failure: Option<Failure>,
}

impl Admission {
    fn result(&self) -> io::Result<()> {
        self.failure.as_ref().map_or(Ok(()), |failure| Err(failure.error()))
    }

    fn fail(&mut self, error: io::Error) -> io::Error {
        if self.failure.is_none() {
            tracing::warn!(%error, "PTY recording stopped; terminal remains active");
            self.failure = Some(Failure {
                kind: error.kind(),
                message: error.to_string(),
            });
        }
        // Never resume after a gap: the accepted prefix is still drained.
        self.sender = None;
        self.failure.as_ref().unwrap().error()
    }
}

/// Shared, nonblocking producers for one private PTY transcript.
///
/// Start only after process confinement; the worker inherits that boundary.
/// Producer timestamps and queue order are captured under a short admission
/// lock which is never held during storage I/O. Saturation stops recording
/// with WouldBlock rather than delaying a terminal. Flush/shutdown report
/// failures; neither they nor Drop join a blocking worker on a Tokio thread.
#[derive(Clone)]
pub struct PtyRecorder {
    admission: Arc<Mutex<Admission>>,
    bytes: Arc<Semaphore>,
    completed: watch::Receiver<bool>,
}

impl PtyRecorder {
    pub fn new(log: PtyLog) -> io::Result<Self> {
        Self::spawn_writer(move |frame| log.write_frame(frame))
    }

    fn spawn_writer(mut write: impl FnMut(&[u8]) -> io::Result<()> + Send + 'static) -> io::Result<Self> {
        let (sender, mut receiver) = mpsc::channel(QUEUE_RECORDS);
        let admission = Arc::new(Mutex::new(Admission {
            sender: Some(sender),
            failure: None,
        }));
        // The worker must not keep its own producer alive after all callers
        // drop: otherwise channel closure and shutdown could never finish.
        let owner = Arc::downgrade(&admission);
        let (done, completed) = watch::channel(false);
        std::thread::Builder::new()
            .name("pty-transcript".into())
            .spawn(move || {
                let mut failed = false;
                while let Some(command) = receiver.blocking_recv() {
                    match command {
                        Command::Record { ref frame, .. } if !failed => {
                            if let Err(error) = write(frame) {
                                failed = true;
                                if let Some(owner) = owner.upgrade() {
                                    owner.lock().unwrap().fail(error);
                                }
                            }
                        }
                        Command::Record { .. } => {}
                        Command::Flush(reply) => {
                            let result = owner.upgrade().map_or_else(
                                || Err(io::Error::from(io::ErrorKind::BrokenPipe)),
                                |owner| owner.lock().unwrap().result(),
                            );
                            let _ = reply.send(result);
                        }
                    }
                }
                // Release the file/directory before acknowledging shutdown.
                drop(write);
                let _ = done.send(true);
            })?;
        Ok(Self {
            admission,
            bytes: Arc::new(Semaphore::new(QUEUE_BYTES)),
            completed,
        })
    }

    pub fn record_output(&self, data: &[u8]) -> io::Result<()> {
        self.record(DIR_OUTPUT, data)
    }

    pub fn record_input(&self, data: &[u8]) -> io::Result<()> {
        self.record(DIR_INPUT, data)
    }

    fn record(&self, direction: u8, data: &[u8]) -> io::Result<()> {
        if data.is_empty() {
            return Ok(());
        }
        let mut admission = self.admission.lock().unwrap();
        admission.result()?;
        let sender = admission.sender.as_ref().ok_or(io::ErrorKind::BrokenPipe)?.clone();
        let size = data.len().saturating_add(13);
        if size > QUEUE_BYTES {
            return Err(admission.fail(io::Error::new(
                io::ErrorKind::WouldBlock,
                "PTY record exceeds memory budget",
            )));
        }
        let bytes = self.bytes.clone().try_acquire_many_owned(size as u32).map_err(|_| {
            admission.fail(io::Error::new(
                io::ErrorKind::WouldBlock,
                "PTY recording memory budget exhausted",
            ))
        })?;
        let slot = sender.try_reserve_owned().map_err(|error| {
            let kind = match error {
                mpsc::error::TrySendError::Full(_) => io::ErrorKind::WouldBlock,
                mpsc::error::TrySendError::Closed(_) => io::ErrorKind::BrokenPipe,
            };
            admission.fail(io::Error::new(kind, "PTY recording queue unavailable"))
        })?;
        slot.send(Command::Record {
            frame: record_frame(direction, data),
            _bytes: bytes,
        });
        drop(admission);
        Ok(())
    }

    /// All previously accepted writes are visible to a reader after this
    /// barrier. This is not an fsync/power-loss durability guarantee.
    pub async fn flush(&self) -> io::Result<()> {
        let (reply, result) = oneshot::channel();
        let sender = self.admission.lock().unwrap().sender.clone();
        let sent = match &sender {
            Some(sender) => sender.send(Command::Flush(reply)).await.is_ok(),
            None => false,
        };
        drop(sender);
        if sent {
            return result.await.map_err(|_| io::Error::from(io::ErrorKind::BrokenPipe))?;
        }
        self.wait_completion().await
    }

    /// Close all shared producers, drain accepted records and release storage.
    /// Safe to call concurrently or retry after cancellation.
    pub async fn shutdown(&self) -> io::Result<()> {
        self.admission.lock().unwrap().sender = None;
        self.wait_completion().await
    }

    async fn wait_completion(&self) -> io::Result<()> {
        let mut completed = self.completed.clone();
        completed
            .wait_for(|done| *done)
            .await
            .map_err(|_| io::Error::from(io::ErrorKind::BrokenPipe))?;
        self.admission.lock().unwrap().result()
    }
}

#[cfg(test)]
mod tests;
