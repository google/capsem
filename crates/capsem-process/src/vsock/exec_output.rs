/// Maximum guest exec output retained in memory.
///
/// The Exec vsock port is a raw stream, so the `MAX_FRAME_SIZE` bound that
/// `read_control_msg` applies to length-prefixed control frames never reaches
/// it. Without a cap here, a guest running `yes` grows this process until the
/// OOM killer takes it and every in-flight job with it.
///
/// 10 MiB matches capsem-gateway's `MAX_BODY_SIZE`: output past that already
/// cannot traverse the gateway to a remote client, so this moves an existing
/// ceiling to before the allocation instead of after it.
pub(super) const MAX_EXEC_OUTPUT_BYTES: usize = 10 * 1024 * 1024;

/// Drain one exec-output stream through EOF, retaining at most
/// [`MAX_EXEC_OUTPUT_BYTES`].
///
/// Returns the retained bytes and the total number of bytes seen, which differ
/// exactly when the guest exceeded the cap. Reading continues past the cap so
/// the guest is not left blocked on a full socket and so the reported total is
/// the real one; only the retained buffer stops growing.
///
/// Signals can interrupt a blocking socket read. `Interrupted` is not EOF:
/// treating it as completion publishes an empty/partial buffer before the
/// guest's `ExecDone`, while still returning the child's successful exit code.
pub(super) fn read_exec_output(reader: &mut impl std::io::Read) -> (Vec<u8>, u64) {
    read_output(reader, |_: &[u8]| Ok(()), false).expect("capture has no fallible forwarding")
}

fn read_output(
    reader: &mut impl std::io::Read,
    mut forward: impl FnMut(&[u8]) -> std::io::Result<()>,
    strict: bool,
) -> std::io::Result<(Vec<u8>, u64)> {
    let mut output = Vec::new();
    let mut total_seen: u64 = 0;
    let mut read_buf = [0u8; 8192];
    loop {
        match reader.read(&mut read_buf) {
            Ok(0) => break,
            Ok(n) => {
                forward(&read_buf[..n])?;
                total_seen = total_seen.saturating_add(n as u64);
                let room = MAX_EXEC_OUTPUT_BYTES.saturating_sub(output.len());
                if room > 0 {
                    output.extend_from_slice(&read_buf[..n.min(room)]);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) if strict => return Err(error),
            Err(_) => break,
        }
    }
    Ok((output, total_seen))
}

pub(super) fn stream_exec_output(
    reader: &mut impl std::io::Read,
    id: u64,
    sender: &tokio::sync::mpsc::Sender<capsem_proto::ipc::ProcessToService>,
) -> std::io::Result<(Vec<u8>, u64)> {
    let mut attached = true;
    read_output(
        reader,
        |data| {
            if attached {
                attached = sender
                    .blocking_send(capsem_proto::ipc::ProcessToService::ExecOutput {
                        id,
                        data: data.to_vec(),
                    })
                    .is_ok();
            }
            // A detached client owns no guest lifetime. Keep draining with the
            // same capture bound so logs cannot block or SIGPIPE the workload.
            Ok(())
        },
        true,
    )
}

#[cfg(test)]
mod tests;
