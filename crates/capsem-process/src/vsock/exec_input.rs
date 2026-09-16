use capsem_core::VsockConnection;
use capsem_proto::ExecInputFrame;

pub(super) fn spawn(
    connection: &VsockConnection,
    input: Option<tokio::sync::mpsc::Receiver<ExecInputFrame>>,
) -> Option<std::thread::JoinHandle<()>> {
    let mut input = input?;
    let mut writer = crate::helpers::clone_fd(connection, "duplicate-exec-input-writer")?;
    Some(std::thread::spawn(move || {
        while let Some(frame) = input.blocking_recv() {
            if capsem_proto::write_exec_input(&mut writer, &frame).is_err() || matches!(frame, ExecInputFrame::StdinEof)
            {
                break;
            }
        }
    }))
}
