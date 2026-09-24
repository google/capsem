use capsem_proto::{ExecOutputChannel, ExecOutputProtocol};
use std::sync::Arc;

use crate::job_store::JobStore;

/// Guest exec output a buffered result returns, both lanes together in the
/// order they arrived. The whole result travels in one `ExecResult` IPC frame,
/// so this must leave the envelope room under that frame's cap.
pub(super) const MAX_EXEC_OUTPUT_BYTES: usize = 10 * 1024 * 1024;

/// Guest exec output retained per lane for the session ledger, whether the
/// exec was buffered or streamed: the logger's own body cap, so what is kept
/// is exactly what the archive can store. A streamed exec's bytes reaching its
/// client says nothing about the ledger; this copy is what the ledger gets.
/// Per exec the capture holds at most two of these.
pub(super) const EXEC_LEDGER_BODY_BYTES: usize = capsem_logger::MAX_BODY_BLOB_BYTES;

const _: () = assert!(
    MAX_EXEC_OUTPUT_BYTES <= EXEC_LEDGER_BODY_BYTES,
    "the buffered result is a prefix of the retained lanes"
);
const _: () = assert!(
    MAX_EXEC_OUTPUT_BYTES + 64 * 1024 <= capsem_foundation::ipc_channel::MAX_IPC_FRAME_SIZE as usize,
    "a full buffered result must fit one IPC frame"
);

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct ExecCapture {
    /// Each lane's leading bytes, exactly as the guest wrote them, up to
    /// `EXEC_LEDGER_BODY_BYTES`.
    pub(super) stdout: Vec<u8>,
    pub(super) stderr: Vec<u8>,
    /// What the guest wrote per lane, retained or not.
    pub(super) stdout_bytes: u64,
    pub(super) stderr_bytes: u64,
    /// Lane lengths at the moment the buffered result's combined
    /// `MAX_EXEC_OUTPUT_BYTES` ran out; `None` while it has not. Until then the
    /// result and the retained lanes are the same bytes, so the result is
    /// always these leading slices of them and never a second buffer.
    pub(super) response_cut: Option<(usize, usize)>,
    /// Why the output stopped early, when it did. A malformed or truncated
    /// frame used to read as a clean EOF: a non-streaming exec then returned
    /// partial output with the guest's exit code and nothing said so.
    pub(super) error: Option<String>,
}

impl ExecCapture {
    /// Count, retain and budget one chunk of guest output.
    fn admit(&mut self, channel: ExecOutputChannel, data: &[u8]) {
        if self.response_cut.is_none() {
            // No lane can reach its ledger cap first: before the cut both
            // lanes together hold less than the result budget.
            let room = MAX_EXEC_OUTPUT_BYTES - (self.stdout.len() + self.stderr.len());
            if data.len() >= room {
                self.response_cut = Some(match channel {
                    ExecOutputChannel::Stdout => (self.stdout.len() + room, self.stderr.len()),
                    ExecOutputChannel::Stderr => (self.stdout.len(), self.stderr.len() + room),
                });
            }
        }
        let (retained, total) = match channel {
            ExecOutputChannel::Stdout => (&mut self.stdout, &mut self.stdout_bytes),
            ExecOutputChannel::Stderr => (&mut self.stderr, &mut self.stderr_bytes),
        };
        *total = total.saturating_add(data.len() as u64);
        let keep = data.len().min(EXEC_LEDGER_BODY_BYTES - retained.len());
        retained.extend_from_slice(&data[..keep]);
    }
}

pub(super) fn deposit(job_store: &JobStore, id: u64, capture: ExecCapture) -> Option<Arc<tokio::sync::Notify>> {
    // One line per exec at the EXEC-port boundary: whether the guest's output
    // reached the host at all, and how much.
    tracing::debug!(
        exec_id = id,
        stdout_bytes = capture.stdout_bytes,
        stderr_bytes = capture.stderr_bytes,
        error = ?capture.error,
        "exec output read"
    );
    let mut active = job_store.active_execs.lock().unwrap();
    let Some(exec) = active.get_mut(&id) else {
        // The exec already completed without this output; say so rather than
        // dropping the bytes without a trace.
        tracing::warn!(
            exec_id = id,
            stdout_bytes = capture.stdout_bytes,
            stderr_bytes = capture.stderr_bytes,
            "exec output arrived after its exec completed; discarding it"
        );
        return None;
    };
    exec.captured = capture.stdout;
    exec.captured_stderr = capture.stderr;
    exec.total_bytes = capture.stdout_bytes;
    exec.stderr_bytes = capture.stderr_bytes;
    exec.response_cut = capture.response_cut;
    if let Some(error) = capture.error {
        exec.output_error.get_or_insert(error);
    }
    let deposited = Arc::clone(&exec.deposited);
    drop(active);
    Some(deposited)
}

/// Drain framed exec output through EOF. Reading continues after the retained
/// cap so the guest cannot block on a full socket and telemetry records the
/// actual byte volume.
#[cfg(test)]
pub(super) fn read_exec_output(reader: &mut impl std::io::Read) -> ExecCapture {
    read_output(reader, |_, _| {})
}

pub(super) fn read_exec_output_protocol(reader: &mut impl std::io::Read, protocol: ExecOutputProtocol) -> ExecCapture {
    read_output_protocol(reader, protocol, |_, _| {})
}

pub(super) fn read_protocol(
    reader: &mut impl std::io::Read,
    id: u64,
    sender: Option<&tokio::sync::mpsc::Sender<capsem_proto::ipc::ProcessToService>>,
    protocol: ExecOutputProtocol,
) -> ExecCapture {
    match sender {
        Some(sender) => stream_exec_output_protocol(reader, id, sender, protocol),
        None => read_exec_output_protocol(reader, protocol),
    }
}

/// Bytes buffered per read of the EXEC socket: frames are read from memory, not
/// one syscall for a length and another for each payload.
const EXEC_OUTPUT_READ_BUFFER: usize = 64 * 1024;

/// A read error ends the capture with its reason and keeps every byte that
/// arrived before it, streamed or not. A streamed read used to return the
/// error alone, and its caller deposited an empty capture: the ledger lost
/// exactly the output of the execs whose streams broke.
fn read_output(reader: &mut impl std::io::Read, mut forward: impl FnMut(ExecOutputChannel, Vec<u8>)) -> ExecCapture {
    let mut reader = std::io::BufReader::with_capacity(EXEC_OUTPUT_READ_BUFFER, reader);
    let mut capture = ExecCapture::default();
    loop {
        let frame = match read_frame(&mut reader) {
            Ok(Some(frame)) => frame,
            Ok(None) => break,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => {
                capture.error = Some(format!("exec output transport failed: {error}"));
                break;
            }
        };
        capture.admit(frame.channel, &frame.data);
        // Retain first, then hand the bytes on: a streamed chunk moves, never clones.
        forward(frame.channel, frame.data);
    }
    capture
}

fn read_output_protocol(
    reader: &mut impl std::io::Read,
    protocol: ExecOutputProtocol,
    forward: impl FnMut(ExecOutputChannel, Vec<u8>),
) -> ExecCapture {
    match protocol {
        ExecOutputProtocol::RawMerged => read_raw_output(reader, forward),
        ExecOutputProtocol::FramedLanes => read_output(reader, forward),
    }
}

fn read_raw_output(
    reader: &mut impl std::io::Read,
    mut forward: impl FnMut(ExecOutputChannel, Vec<u8>),
) -> ExecCapture {
    let mut capture = ExecCapture::default();
    let mut buffer = vec![0_u8; EXEC_OUTPUT_READ_BUFFER];
    loop {
        let read = match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => {
                capture.error = Some(format!("exec output transport failed: {error}"));
                break;
            }
        };
        capture.admit(ExecOutputChannel::Stdout, &buffer[..read]);
        forward(ExecOutputChannel::Stdout, buffer[..read].to_vec());
    }
    capture
}

/// Distinguish clean socket EOF between frames from a truncated frame: an
/// empty buffer after a refill is EOF; anything else must be a whole frame.
fn read_frame(reader: &mut impl std::io::BufRead) -> std::io::Result<Option<capsem_proto::ExecOutputFrame>> {
    loop {
        match reader.fill_buf() {
            Ok([]) => return Ok(None),
            Ok(_) => return capsem_proto::read_exec_output(reader).map(Some),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
}

#[cfg(test)]
pub(super) fn stream_exec_output(
    reader: &mut impl std::io::Read,
    id: u64,
    sender: &tokio::sync::mpsc::Sender<capsem_proto::ipc::ProcessToService>,
) -> ExecCapture {
    let mut attached = true;
    read_output(reader, |channel, data| {
        if attached {
            attached = sender
                .blocking_send(capsem_proto::ipc::ProcessToService::ExecOutput { id, channel, data })
                .is_ok();
        }
    })
}

pub(super) fn stream_exec_output_protocol(
    reader: &mut impl std::io::Read,
    id: u64,
    sender: &tokio::sync::mpsc::Sender<capsem_proto::ipc::ProcessToService>,
    protocol: ExecOutputProtocol,
) -> ExecCapture {
    let mut attached = true;
    read_output_protocol(reader, protocol, |channel, data| {
        if attached {
            attached = sender
                .blocking_send(capsem_proto::ipc::ProcessToService::ExecOutput { id, channel, data })
                .is_ok();
        }
    })
}

#[cfg(test)]
mod tests;
