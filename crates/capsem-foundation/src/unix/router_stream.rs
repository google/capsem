//! Bounded duplex copying shared by the router and the guest bridge.
pub use capsem_proto::router::{CloseReason, CloseReport};
use std::future::Future;
use std::io;
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering::Relaxed};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::time::timeout;

pub const BUFFER_SIZE: usize = 16 * 1024;
pub const SOCKET_BUFFER_SIZE: usize = 64 * 1024;

#[derive(Clone, Copy)]
pub struct Limits {
    pub write_stall: Duration,
    pub half_close: Duration,
    /// How long a write may make no progress before the copy says so. The
    /// stall limit still decides when it gives up; this only makes the wait
    /// visible while it is happening, which the close report never can.
    pub stall_report: Duration,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            write_stall: Duration::from_secs(60),
            half_close: Duration::from_secs(60),
            stall_report: Duration::from_secs(5),
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

/// How an endpoint carries the end of each direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Framing {
    /// The socket's own half-close is the signal.
    Raw,
    /// Big-endian `u32` length-prefixed frames, a zero length ending that
    /// direction, and never a socket shutdown. This is how every VSOCK leg
    /// carries it: Apple VZ lets a shutdown overtake bytes still in flight,
    /// so a half-close there truncated or stalled the stream behind it.
    Framed,
}

/// The framing of both ends of one copy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Framings {
    pub source: Framing,
    pub destination: Framing,
}

impl Framings {
    pub const RAW: Self = Self {
        source: Framing::Raw,
        destination: Framing::Raw,
    };
}

const FRAME_HEADER: usize = 4;

pub async fn copy(
    source: &mut (impl AsyncRead + AsyncWrite + Unpin),
    destination: &mut (impl AsyncRead + AsyncWrite + Unpin),
    framings: Framings,
    limits: Limits,
) -> Outcome {
    copy_until(source, destination, framings, limits, std::future::pending()).await
}

/// Cooperative termination retains delivered counts, including during FIN drain.
pub async fn copy_until(
    source: &mut (impl AsyncRead + AsyncWrite + Unpin),
    destination: &mut (impl AsyncRead + AsyncWrite + Unpin),
    framings: Framings,
    limits: Limits,
    stop: impl Future<Output = ()>,
) -> Outcome {
    let (source_read, source_write) = tokio::io::split(source);
    let (destination_read, destination_write) = tokio::io::split(destination);
    let progress = Progress::default();
    let result = {
        let forward = direction(
            source_read,
            destination_write,
            (framings.source, framings.destination),
            limits,
            &progress.sides[FORWARD],
        );
        let reverse = direction(
            destination_read,
            source_write,
            (framings.destination, framings.source),
            limits,
            &progress.sides[REVERSE],
        );
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
            never = watch(&progress, limits.stall_report) => match never {},
        }
    };
    let (reason, error) = match result {
        None => (CloseReason::Cancelled, None),
        Some(Ok(())) => (CloseReason::Complete, None),
        Some(Err((reason, error))) => (reason, Some(error)),
    };
    Outcome {
        from_source: progress.sides[FORWARD].copied.load(Relaxed),
        to_source: progress.sides[REVERSE].copied.load(Relaxed),
        reason,
        error,
    }
}

const FORWARD: usize = 0;
const REVERSE: usize = 1;
const READING: u8 = 0;
const WRITING: u8 = 1;
const DONE: u8 = 2;

/// What each direction is doing, published for the stall watch alone.
/// Relaxed is enough: the watch only reports, and a stale read of a moving
/// counter is a direction that is not stalled.
#[derive(Default)]
struct Progress {
    sides: [Side; 2],
}

#[derive(Default)]
struct Side {
    phase: AtomicU8,
    copied: AtomicU64,
}

impl Side {
    fn phase(&self) -> &'static str {
        match self.phase.load(Relaxed) {
            READING => "read",
            WRITING => "write",
            _ => "done",
        }
    }
}

/// Report, once per episode, a direction that sat in one write across a
/// whole interval. A peer that never drains ends at the stall limit either
/// way; before that, only this says which leg stopped and what the other
/// was doing, both of which the close report loses.
async fn watch(progress: &Progress, every: Duration) -> std::convert::Infallible {
    let mut interval = tokio::time::interval_at(tokio::time::Instant::now() + every, every);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut last = [(DONE, 0); 2];
    let mut reported = [false; 2];
    loop {
        interval.tick().await;
        for side in [FORWARD, REVERSE] {
            let now = (
                progress.sides[side].phase.load(Relaxed),
                progress.sides[side].copied.load(Relaxed),
            );
            let stalled = now.0 == WRITING && now == last[side];
            if stalled && !reported[side] {
                let [forward, reverse] = &progress.sides;
                tracing::warn!(
                    stalled = if side == FORWARD { "forward" } else { "reverse" },
                    stalled_for_ms = every.as_millis() as u64,
                    forward_phase = forward.phase(),
                    forward_bytes = forward.copied.load(Relaxed),
                    reverse_phase = reverse.phase(),
                    reverse_bytes = reverse.copied.load(Relaxed),
                    "stream write made no progress"
                );
            }
            reported[side] = stalled;
            last[side] = now;
        }
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
// A framed reader is decoded in place and a framed writer gets its header in
// the slot ahead of the bytes read, so neither framing copies a payload.
async fn direction(
    reader: impl AsyncRead + Unpin,
    writer: impl AsyncWrite + Unpin,
    (read_framing, write_framing): (Framing, Framing),
    limits: Limits,
    side: &Side,
) -> Result<(), (CloseReason, io::Error)> {
    let result = copy_direction(reader, writer, (read_framing, write_framing), limits, side).await;
    side.phase.store(DONE, Relaxed);
    result
}

async fn copy_direction(
    mut reader: impl AsyncRead + Unpin,
    mut writer: impl AsyncWrite + Unpin,
    (read_framing, write_framing): (Framing, Framing),
    limits: Limits,
    side: &Side,
) -> Result<(), (CloseReason, io::Error)> {
    let header = if write_framing == Framing::Framed {
        FRAME_HEADER
    } else {
        0
    };
    let mut buffer = vec![0; header + BUFFER_SIZE].into_boxed_slice();
    let mut frames = FrameDecoder::default();
    loop {
        side.phase.store(READING, Relaxed);
        let count = reader.read(&mut buffer[header..]).await.map_err(io_failure)?;
        side.phase.store(WRITING, Relaxed);
        if count == 0 {
            if read_framing == Framing::Framed {
                return Err(io_failure(io::Error::new(
                    io::ErrorKind::ConnectionReset,
                    "framed stream ended without its end-of-stream frame",
                )));
            }
            return end(&mut writer, write_framing, limits).await;
        }
        let read = header..header + count;
        if read_framing == Framing::Raw {
            send(&mut writer, &mut buffer, read, write_framing, limits).await?;
            side.copied.fetch_add(count as u64, Relaxed);
            continue;
        }
        let mut at = read.start;
        while at < read.end {
            match frames.next(&buffer[at..read.end]) {
                Decoded::Header(used) => at += used,
                Decoded::Payload(length) => {
                    let payload = at..at + length;
                    send(&mut writer, &mut buffer, payload, write_framing, limits).await?;
                    side.copied.fetch_add(length as u64, Relaxed);
                    at += length;
                }
                Decoded::End(used) => {
                    if at + used != read.end {
                        return Err(io_failure(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "bytes after the end-of-stream frame",
                        )));
                    }
                    return end(&mut writer, write_framing, limits).await;
                }
            }
        }
    }
}

/// Write `range` of `buffer`, framed if the writer is: its header goes in the
/// bytes just before the range, which the caller reserved or already consumed.
async fn send(
    writer: &mut (impl AsyncWrite + Unpin),
    buffer: &mut [u8],
    range: std::ops::Range<usize>,
    framing: Framing,
    limits: Limits,
) -> Result<(), (CloseReason, io::Error)> {
    let start = match framing {
        Framing::Raw => range.start,
        Framing::Framed => {
            // A decoded payload reuses bytes it has already read past; a raw
            // read left the reserved slot. Either way the header fits.
            let start = range.start - FRAME_HEADER;
            buffer[start..range.start].copy_from_slice(&(range.len() as u32).to_be_bytes());
            start
        }
    };
    write_all(writer, &buffer[start..range.end], limits).await
}

async fn end(
    writer: &mut (impl AsyncWrite + Unpin),
    framing: Framing,
    limits: Limits,
) -> Result<(), (CloseReason, io::Error)> {
    match framing {
        Framing::Raw => timeout(limits.write_stall, writer.shutdown())
            .await
            .map_err(|_| stalled())?
            .map_err(io_failure),
        Framing::Framed => write_all(writer, &[0; FRAME_HEADER], limits).await,
    }
}

async fn write_all(
    writer: &mut (impl AsyncWrite + Unpin),
    mut remaining: &[u8],
    limits: Limits,
) -> Result<(), (CloseReason, io::Error)> {
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
        remaining = &remaining[written..];
    }
    Ok(())
}

/// Where a framed reader is between frames, across reads of any size.
#[derive(Default)]
struct FrameDecoder {
    header: [u8; FRAME_HEADER],
    header_filled: usize,
    payload_left: usize,
}

enum Decoded {
    /// This many bytes went into a header that is not complete, or that
    /// announced a payload still to come.
    Header(usize),
    /// The next this-many bytes are payload.
    Payload(usize),
    /// A zero-length frame, whose header ended after this many bytes.
    End(usize),
}

impl FrameDecoder {
    fn next(&mut self, bytes: &[u8]) -> Decoded {
        if self.payload_left > 0 {
            let length = self.payload_left.min(bytes.len());
            self.payload_left -= length;
            return Decoded::Payload(length);
        }
        let used = (FRAME_HEADER - self.header_filled).min(bytes.len());
        self.header[self.header_filled..self.header_filled + used].copy_from_slice(&bytes[..used]);
        self.header_filled += used;
        if self.header_filled < FRAME_HEADER {
            return Decoded::Header(used);
        }
        self.header_filled = 0;
        self.payload_left = u32::from_be_bytes(self.header) as usize;
        if self.payload_left == 0 {
            Decoded::End(used)
        } else {
            Decoded::Header(used)
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
