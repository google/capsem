use capsem_proto::ExecOutputChannel;
use std::sync::Arc;

use crate::job_store::JobStore;

/// Maximum combined guest exec output retained in memory.
pub(super) const MAX_EXEC_OUTPUT_BYTES: usize = 10 * 1024 * 1024;

/// Output kept per lane for the exec ledger preview. A streamed exec delivers
/// its output to the client, so this is all it retains.
pub(super) const EXEC_LEDGER_PREVIEW_BYTES: usize = 1024;

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct ExecCapture {
    pub(super) stdout: Vec<u8>,
    pub(super) stderr: Vec<u8>,
    pub(super) stdout_bytes: u64,
    pub(super) stderr_bytes: u64,
    /// Why the output stopped early, when it did. A malformed or truncated
    /// frame used to read as a clean EOF: a non-streaming exec then returned
    /// partial output with the guest's exit code and nothing said so.
    pub(super) error: Option<String>,
}

pub(super) fn deposit(job_store: &JobStore, id: u64, capture: ExecCapture) -> Option<Arc<tokio::sync::Notify>> {
    let mut active = job_store.active_execs.lock().unwrap();
    let exec = active.get_mut(&id)?;
    exec.captured = capture.stdout;
    exec.captured_stderr = capture.stderr;
    exec.total_bytes = capture.stdout_bytes;
    exec.stderr_bytes = capture.stderr_bytes;
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
pub(super) fn read_exec_output(reader: &mut impl std::io::Read) -> ExecCapture {
    read_output(reader, |_, _| Ok(()), false, MAX_EXEC_OUTPUT_BYTES).expect("capture has no fallible forwarding")
}

fn read_output(
    reader: &mut impl std::io::Read,
    mut forward: impl FnMut(ExecOutputChannel, &[u8]) -> std::io::Result<()>,
    strict: bool,
    retain_per_lane: usize,
) -> std::io::Result<ExecCapture> {
    let mut capture = ExecCapture::default();
    loop {
        let frame = match read_frame(reader) {
            Ok(Some(frame)) => frame,
            Ok(None) => break,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) if strict => return Err(error),
            Err(error) => {
                capture.error = Some(format!("exec output transport failed: {error}"));
                break;
            }
        };
        forward(frame.channel, &frame.data)?;
        let combined = capture.stdout.len().saturating_add(capture.stderr.len());
        let combined_room = MAX_EXEC_OUTPUT_BYTES.saturating_sub(combined);
        let (retained, total) = match frame.channel {
            ExecOutputChannel::Stdout => (&mut capture.stdout, &mut capture.stdout_bytes),
            ExecOutputChannel::Stderr => (&mut capture.stderr, &mut capture.stderr_bytes),
        };
        *total = total.saturating_add(frame.data.len() as u64);
        let lane_room = retain_per_lane.saturating_sub(retained.len());
        let keep = frame.data.len().min(combined_room).min(lane_room);
        retained.extend_from_slice(&frame.data[..keep]);
    }
    Ok(capture)
}

/// Distinguish clean socket EOF between frames from a truncated frame. The
/// shared codec reads a complete four-byte header, so consume one byte here
/// before delegating and prepend it back through `chain`.
fn read_frame(reader: &mut impl std::io::Read) -> std::io::Result<Option<capsem_proto::ExecOutputFrame>> {
    let mut first = [0_u8; 1];
    loop {
        match reader.read(&mut first) {
            Ok(0) => return Ok(None),
            Ok(1) => {
                let mut frame = std::io::Read::chain(first.as_slice(), reader);
                return capsem_proto::read_exec_output(&mut frame).map(Some);
            }
            Ok(_) => unreachable!("one-byte read returned more than one byte"),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
}

pub(super) fn stream_exec_output(
    reader: &mut impl std::io::Read,
    id: u64,
    sender: &tokio::sync::mpsc::Sender<capsem_proto::ipc::ProcessToService>,
) -> std::io::Result<ExecCapture> {
    let mut attached = true;
    read_output(
        reader,
        |channel, data| {
            if attached {
                attached = sender
                    .blocking_send(capsem_proto::ipc::ProcessToService::ExecOutput {
                        id,
                        channel,
                        data: data.to_vec(),
                    })
                    .is_ok();
            }
            Ok(())
        },
        true,
        EXEC_LEDGER_PREVIEW_BYTES,
    )
}

#[cfg(test)]
mod tests;
