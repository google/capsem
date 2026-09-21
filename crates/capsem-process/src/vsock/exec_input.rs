use capsem_core::VsockConnection;
use capsem_proto::ipc::ProcessToService;
use capsem_proto::{ExecInputFrame, ExecOutputProtocol};

pub(super) fn spawn_for(
    connection: &VsockConnection,
    id: u64,
    input: Option<tokio::sync::mpsc::Receiver<ExecInputFrame>>,
    credit: Option<tokio::sync::mpsc::Sender<ProcessToService>>,
    protocol: ExecOutputProtocol,
) -> Option<std::thread::JoinHandle<()>> {
    (protocol == ExecOutputProtocol::FramedLanes)
        .then(|| spawn(connection, id, input, credit))
        .flatten()
}

/// Write queued stdin frames to the guest. Each frame that leaves the queue
/// returns one unit of stdin credit to the service through `credit`.
pub(super) fn spawn(
    connection: &VsockConnection,
    id: u64,
    input: Option<tokio::sync::mpsc::Receiver<ExecInputFrame>>,
    credit: Option<tokio::sync::mpsc::Sender<ProcessToService>>,
) -> Option<std::thread::JoinHandle<()>> {
    let mut input = input?;
    let mut writer = crate::helpers::clone_fd(connection, "duplicate-exec-input-writer")?;
    Some(std::thread::spawn(move || {
        while let Some(frame) = input.blocking_recv() {
            let written = capsem_proto::write_exec_input(&mut writer, &frame);
            if let Some(credit) = &credit {
                // A closed service connection ends the exec anyway.
                let _ = credit.blocking_send(ProcessToService::ExecInputConsumed { id });
            }
            if written.is_err() || matches!(frame, ExecInputFrame::StdinEof) {
                break;
            }
        }
    }))
}

#[cfg(test)]
mod tests;
