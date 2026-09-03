//! Driver between `AggregatorClient` callers and the aggregator subprocess.
//!
//! Callers hand the driver `(request, oneshot)` pairs over an mpsc channel.
//! The writer task frames each request onto the subprocess stdin and parks
//! the oneshot in the pending map; the reader task frames responses off the
//! subprocess stdout and routes each to its parked oneshot by `id`.
//!
//! The pending map is the only place a caller can get stuck. Two exits keep
//! it honest: when the subprocess stdout closes, every parked oneshot is
//! dropped so its caller fails at once instead of waiting out the endpoint
//! timeout, and the map is closed so later requests fail the same way. And a
//! caller that gave up (its receiver dropped) is pruned on the next insert, so
//! a remote MCP server that never answers cannot grow the map for the life of
//! the VM.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use capsem_proto::mcp_aggregator::{read_frame, write_frame, AggregatorResponse, ClientMessage};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{mpsc, oneshot};
use tracing::{error, info};

#[derive(Default)]
struct Pending {
    waiters: HashMap<u64, oneshot::Sender<AggregatorResponse>>,
    closed: bool,
}

impl Pending {
    /// Fail every parked caller and refuse new ones. Dropping a sender wakes
    /// its caller with "aggregator response channel dropped".
    fn close(&mut self) {
        self.closed = true;
        self.waiters.clear();
    }

    /// Park a caller unless the subprocess is gone. Returns false, dropping
    /// the sender, when the request cannot be answered.
    fn park(&mut self, id: u64, tx: oneshot::Sender<AggregatorResponse>) -> bool {
        if self.closed {
            return false;
        }
        self.waiters.retain(|_, waiter| !waiter.is_closed());
        self.waiters.insert(id, tx);
        true
    }
}

/// The callers parked between writing their request and reading its response.
pub(crate) struct Inflight(Arc<Mutex<Pending>>);

impl Inflight {
    /// Fail every parked caller and refuse new ones. The reader does this on
    /// EOF; the child monitor does it on exit, for the case where a grandchild
    /// inherited the stdout pipe and EOF never comes.
    pub(crate) fn close(&self) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).close();
    }

    #[cfg(test)]
    pub(crate) fn count(&self) -> usize {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).waiters.len()
    }
}

/// Spawn the reader and writer tasks for one aggregator subprocess.
pub(crate) fn spawn<W, R>(mut rx: mpsc::Receiver<ClientMessage>, mut stdin: W, mut stdout: R) -> Inflight
where
    W: AsyncWrite + Unpin + Send + 'static,
    R: AsyncRead + Unpin + Send + 'static,
{
    let pending = Arc::new(Mutex::new(Pending::default()));

    let pending_reader = Arc::clone(&pending);
    tokio::spawn(async move {
        info!("aggregator reader task started");
        loop {
            match read_frame::<_, AggregatorResponse>(&mut stdout).await {
                Ok(Some(resp)) => {
                    let waiter = pending_reader
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .waiters
                        .remove(&resp.id);
                    if let Some(tx) = waiter {
                        capsem_core::try_send!("aggregator_oneshot", tx.send(resp));
                    }
                }
                Ok(None) => {
                    info!("aggregator stdout closed (EOF)");
                    break;
                }
                Err(e) => {
                    error!(error = %e, "failed to read aggregator response frame");
                    break;
                }
            }
        }
        pending_reader.lock().unwrap_or_else(|e| e.into_inner()).close();
        info!("aggregator reader task ending; in-flight callers failed");
    });

    let pending_writer = Arc::clone(&pending);
    tokio::spawn(async move {
        info!("aggregator writer task started");
        while let Some((req, resp_tx)) = rx.recv().await {
            if !pending_writer
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .park(req.id, resp_tx)
            {
                break;
            }
            if let Err(e) = write_frame(&mut stdin, &req).await {
                error!(error = %e, "failed to write aggregator request frame");
                pending_writer.lock().unwrap_or_else(|e| e.into_inner()).close();
                break;
            }
        }
        info!("aggregator writer task ending");
    });

    Inflight(pending)
}

#[cfg(test)]
mod tests;
