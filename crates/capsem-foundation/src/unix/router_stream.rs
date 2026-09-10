//! Bounded duplex copying shared by the router and the guest bridge.
pub use capsem_proto::router::{CloseReason, CloseReport};
use std::future::Future;
use std::io;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::time::timeout;

pub const BUFFER_SIZE: usize = 16 * 1024;

#[derive(Clone, Copy)]
pub struct Limits {
    pub write_stall: Duration,
    pub half_close: Duration,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            write_stall: Duration::from_secs(60),
            half_close: Duration::from_secs(60),
        }
    }
}

#[derive(Debug)]
#[must_use]
pub struct Outcome {
    pub from_source: u64,
    pub to_source: u64,
    pub reason: CloseReason,
    pub error: Option<io::Error>,
}

impl Outcome {
    pub fn report(&self) -> CloseReport {
        CloseReport {
            reason: self.reason,
            from_source: self.from_source,
            to_source: self.to_source,
        }
    }
}

pub async fn copy(
    source: &mut (impl AsyncRead + AsyncWrite + Unpin),
    destination: &mut (impl AsyncRead + AsyncWrite + Unpin),
    limits: Limits,
) -> Outcome {
    copy_until(source, destination, limits, std::future::pending()).await
}

/// Cooperative termination retains delivered counts, including during FIN drain.
pub async fn copy_until(
    source: &mut (impl AsyncRead + AsyncWrite + Unpin),
    destination: &mut (impl AsyncRead + AsyncWrite + Unpin),
    limits: Limits,
    stop: impl Future<Output = ()>,
) -> Outcome {
    let (source_read, source_write) = tokio::io::split(source);
    let (destination_read, destination_write) = tokio::io::split(destination);
    let (mut from_source, mut to_source) = (0, 0);
    let result = {
        let forward = direction(source_read, destination_write, limits, &mut from_source);
        let reverse = direction(destination_read, source_write, limits, &mut to_source);
        tokio::pin!(forward, reverse);
        let forwarding = async {
            tokio::select! {
                result = &mut forward => finish(result, reverse, limits.half_close).await,
                result = &mut reverse => finish(result, forward, limits.half_close).await,
            }
        };
        tokio::select! {
            biased;
            _ = stop => None,
            result = forwarding => Some(result),
        }
    };
    let (reason, error) = match result {
        None => (CloseReason::Cancelled, None),
        Some(Ok(())) => (CloseReason::Complete, None),
        Some(Err((reason, error))) => (reason, Some(error)),
    };
    Outcome {
        from_source,
        to_source,
        reason,
        error,
    }
}

async fn finish(
    first: Result<(), (CloseReason, io::Error)>,
    remaining: impl Future<Output = Result<(), (CloseReason, io::Error)>>,
    deadline: Duration,
) -> Result<(), (CloseReason, io::Error)> {
    first?;
    timeout(deadline, remaining).await.unwrap_or_else(|_| {
        Err((
            CloseReason::HalfCloseTimeout,
            io::Error::new(io::ErrorKind::TimedOut, "peer did not finish after half-close"),
        ))
    })
}

// No read deadline: a quiet connection is healthy. Each successful write renews
// the stall budget, and the next read waits until this fixed buffer is drained.
async fn direction(
    mut reader: impl AsyncRead + Unpin,
    mut writer: impl AsyncWrite + Unpin,
    limits: Limits,
    copied: &mut u64,
) -> Result<(), (CloseReason, io::Error)> {
    let mut buffer = vec![0; BUFFER_SIZE].into_boxed_slice();
    loop {
        let count = reader.read(&mut buffer).await.map_err(io_failure)?;
        if count == 0 {
            return timeout(limits.write_stall, writer.shutdown())
                .await
                .map_err(|_| stalled())?
                .map_err(io_failure);
        }
        let mut remaining = &buffer[..count];
        while !remaining.is_empty() {
            let written = timeout(limits.write_stall, writer.write(remaining))
                .await
                .map_err(|_| stalled())?
                .map_err(io_failure)?;
            if written == 0 {
                return Err(io_failure(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "stream stopped accepting bytes",
                )));
            }
            *copied += written as u64;
            remaining = &remaining[written..];
        }
    }
}

fn stalled() -> (CloseReason, io::Error) {
    (
        CloseReason::WriteStall,
        io::Error::new(io::ErrorKind::TimedOut, "stream write made no progress"),
    )
}

fn io_failure(error: io::Error) -> (CloseReason, io::Error) {
    let reason = match error.kind() {
        io::ErrorKind::ConnectionReset | io::ErrorKind::ConnectionAborted | io::ErrorKind::BrokenPipe => {
            CloseReason::Reset
        }
        _ => CloseReason::Io,
    };
    (reason, error)
}

#[cfg(test)]
mod tests;
