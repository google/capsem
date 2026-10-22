//! The session terminal's transport. Byte interpretation belongs to core.
use super::*;
use std::collections::VecDeque;
use std::os::fd::AsFd;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const SERIAL_LOG_QUEUE_CAPACITY: usize = 8;

struct TerminalSocket {
    anchor: Arc<VsockConnection>,
    reader: tokio::net::unix::OwnedReadHalf,
    writer: tokio::net::unix::OwnedWriteHalf,
}

impl TerminalSocket {
    fn open(anchor: Arc<VsockConnection>) -> std::io::Result<Self> {
        let fd = anchor.try_clone_fd()?;
        capsem_foundation::unix::fd::set_nonblocking(fd.as_fd(), true)?;
        let stream = tokio::net::UnixStream::from_std(std::os::unix::net::UnixStream::from(fd))?;
        let (reader, writer) = stream.into_split();
        Ok(Self { anchor, reader, writer })
    }
}

impl Drop for TerminalSocket {
    fn drop(&mut self) {
        if let Err(error) = self.anchor.shutdown_both() {
            if error.kind() != std::io::ErrorKind::NotConnected {
                warn!(operation = "shutdown-terminal-vsock", error = %error, "terminal shutdown failed");
            }
        }
    }
}

async fn deadline(at: Option<tokio::time::Instant>) {
    match at {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}

pub(crate) struct TerminalBridge {
    stop: tokio::sync::oneshot::Sender<()>,
    task: tokio::task::JoinHandle<()>,
}

impl TerminalBridge {
    pub(crate) async fn shutdown(self) -> Result<(), tokio::task::JoinError> {
        let _ = self.stop.send(());
        self.task.await
    }
}

pub(super) fn spawn(
    terminal: Arc<capsem_core::terminal::SessionTerminal>,
    mut rekey: mpsc::Receiver<Arc<VsockConnection>>,
    mut input: mpsc::Receiver<Vec<u8>>,
    pty_log: Option<capsem_core::pty_log::PtyRecorder>,
    serial_log: impl std::io::Write + Send + 'static,
) -> TerminalBridge {
    let (log, log_handle) = spawn_serial_log_writer(move || Ok(serial_log));
    let (stop, mut stopped) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let mut active: Option<TerminalSocket> = None;
        let mut coalesce = capsem_core::vm::vsock::CoalesceBuffer::new();
        let mut coalesce_at = None;
        let mut pending_log = None;
        let mut logging = true;
        let mut input_open = true;
        let mut outgoing = VecDeque::<Vec<u8>>::new();
        let mut written = 0;
        // One input frame may be pending. Once replies appear, stop reading
        // until they drain, so a guest that never reads cannot grow a queue.
        // An outstanding synchronized update can add one final bounded reply.
        let mut replies_pending = false;
        let mut buffer = vec![0u8; 65536];
        loop {
            let mut flush = false;
            let mut replace = None;
            let mut finish = false;
            let connected = active.is_some();
            let sync_at = terminal.sync_deadline().map(tokio::time::Instant::from_std);
            let (reader, writer) = active.as_mut().map(|c| (&mut c.reader, &mut c.writer)).unzip();
            let remaining = outgoing.front().map_or(&[][..], |bytes| &bytes[written..]);
            // read/write are cancellation safe. In particular, never restart a
            // write_all after a timer or reconnect: retain its exact cursor.
            tokio::select! {
                _ = &mut stopped => {
                    finish = true;
                    replace = Some(None);
                    flush = true;
                }
                connection = rekey.recv() => {
                    finish = connection.is_none();
                    replace = Some(connection);
                    flush = true;
                }
                _ = deadline(sync_at) => {
                    let replies = terminal.expire_sync();
                    // A timeout can expire between sockets. Its new replies
                    // have never been written and belong on the next socket.
                    if !replies.is_empty() {
                        replies_pending = true;
                        outgoing.push_back(replies);
                    }
                }
                _ = deadline(coalesce_at), if pending_log.is_none() => flush = true,
                permit = log.reserve(), if pending_log.is_some() => {
                    match permit {
                        Ok(permit) => permit.send(pending_log.take().unwrap()),
                        Err(error) => {
                            warn!(%error, "terminal serial log writer disconnected");
                            pending_log = None;
                            logging = false;
                        }
                    }
                }
                result = async {
                    match writer {
                        Some(writer) => writer.write(remaining).await,
                        None => std::future::pending().await,
                    }
                }, if !outgoing.is_empty() => {
                    match result {
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                        Ok(0) | Err(_) => { replace = Some(None); flush = true; }
                        Ok(count) => {
                            written += count;
                            if written == outgoing.front().unwrap().len() {
                                outgoing.pop_front();
                                written = 0;
                                if outgoing.is_empty() { replies_pending = false; }
                            }
                        }
                    }
                }
                result = async {
                    match reader {
                        Some(reader) => reader.read(&mut buffer).await,
                        None => std::future::pending().await,
                    }
                }, if pending_log.is_none() && !replies_pending => {
                    match result {
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                        Ok(0) | Err(_) => { replace = Some(None); flush = true; }
                        Ok(count) => {
                            if coalesce.is_empty() {
                                coalesce_at = Some(tokio::time::Instant::now() +
                                    std::time::Duration::from_millis(coalesce.window_ms()));
                            }
                            flush = coalesce.push(&buffer[..count]);
                        }
                    }
                }
                data = input.recv(), if input_open && connected && outgoing.is_empty() => {
                    match data {
                        Some(data) if !data.is_empty() => outgoing.push_back(data),
                        Some(_) => {}
                        None => input_open = false,
                    }
                }
            }
            if flush && !coalesce.is_empty() {
                let batch = coalesce.flush_to(<[u8]>::to_vec);
                let replies = terminal.publish(&batch);
                if !replies.is_empty() && active.is_some() {
                    replies_pending = true;
                    outgoing.push_back(replies);
                }
                if let Some(ref pty_log) = pty_log {
                    // Failure is reported once by the optional storage owner.
                    let _ = pty_log.record_output(&batch);
                }
                if logging {
                    pending_log = Some(batch);
                }
                coalesce_at = None;
            }
            if let Some(connection) = replace {
                // A connection is a byte stream with its own partial writes.
                // Never replay an ambiguous prefix or suffix on a replacement.
                if let Some(old) = active.take() {
                    drop(old);
                    outgoing.clear();
                    written = 0;
                    replies_pending = false;
                }
                match connection {
                    Some(connection) => match TerminalSocket::open(connection) {
                        Ok(connection) => {
                            active = Some(connection);
                            info!("terminal bridge: active");
                        }
                        Err(error) => warn!(%error, "terminal bridge could not adopt vsock"),
                    },
                    None if finish => break,
                    None => {}
                }
            }
        }
        if let Some(batch) = pending_log {
            let _ = log.send(batch).await;
        }
        drop(log);
        let _ = tokio::task::spawn_blocking(move || log_handle.join()).await;
    });
    TerminalBridge { stop, task }
}

pub(super) fn spawn_serial_log_writer<W, F>(open: F) -> (mpsc::Sender<Vec<u8>>, std::thread::JoinHandle<()>)
where
    W: std::io::Write + Send + 'static,
    F: FnOnce() -> std::io::Result<W> + Send + 'static,
{
    let (tx, mut rx) = mpsc::channel::<Vec<u8>>(SERIAL_LOG_QUEUE_CAPACITY);
    let handle = std::thread::spawn(move || {
        let mut writer = match open() {
            Ok(writer) => writer,
            Err(error) => {
                warn!(error = %error, "failed to open serial log");
                return;
            }
        };
        while let Some(batch) = rx.blocking_recv() {
            if let Err(error) = writer.write_all(&batch) {
                warn!(%error, "serial log recording stopped after storage failure");
                return;
            }
        }
        if let Err(error) = writer.flush() {
            warn!(%error, "serial log flush failed");
        }
    });
    (tx, handle)
}

#[cfg(test)]
mod tests;
