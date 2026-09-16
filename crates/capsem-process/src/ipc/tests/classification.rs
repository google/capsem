use super::super::*;

/// Maps a service message to the action category it triggers so the dispatcher
/// contract remains exhaustive as the owner protocol grows.
pub(super) fn classify_ipc_message(msg: &ServiceToProcess) -> IpcAction {
    match msg {
        ServiceToProcess::StartTerminalStream | ServiceToProcess::StopTerminalStream => IpcAction::StreamSetup,
        ServiceToProcess::Ping => IpcAction::HealthCheck,
        ServiceToProcess::TerminalInput { .. } | ServiceToProcess::TerminalResize { .. } => IpcAction::Forward,
        ServiceToProcess::Exec { .. }
        | ServiceToProcess::ExecStream { .. }
        | ServiceToProcess::ExecStreamInput { .. }
        | ServiceToProcess::ExecStreamCloseStdin { .. }
        | ServiceToProcess::CancelExec { .. }
        | ServiceToProcess::PublishPort { .. }
        | ServiceToProcess::DeclarePreview { .. }
        | ServiceToProcess::RevokeExposure { .. }
        | ServiceToProcess::ListPublications { .. }
        | ServiceToProcess::CreatePreviewSession { .. }
        | ServiceToProcess::ExchangePreviewBootstrap { .. }
        | ServiceToProcess::AdmitPreviewConnection { .. }
        | ServiceToProcess::AdmitContainerPull { .. }
        | ServiceToProcess::LinkAttach { .. }
        | ServiceToProcess::LinkDetach { .. }
        | ServiceToProcess::WriteFile { .. }
        | ServiceToProcess::ReadFile { .. }
        | ServiceToProcess::LogFileBoundary { .. }
        | ServiceToProcess::McpListServers { .. }
        | ServiceToProcess::McpListTools { .. }
        | ServiceToProcess::McpRefreshTools { .. }
        | ServiceToProcess::McpCallTool { .. }
        | ServiceToProcess::SnapshotStatus { .. } => IpcAction::Job,
        ServiceToProcess::ConnectPort { .. }
        | ServiceToProcess::AbortPorts { .. }
        | ServiceToProcess::PlugCable { .. }
        | ServiceToProcess::UnplugCable { .. }
        | ServiceToProcess::PrepareSnapshot
        | ServiceToProcess::Unfreeze
        | ServiceToProcess::Resume => IpcAction::Unexpected,
        ServiceToProcess::ReloadConfig => IpcAction::Reload,
        ServiceToProcess::Shutdown | ServiceToProcess::Suspend { .. } => IpcAction::Lifecycle,
    }
}

#[derive(Debug, PartialEq)]
pub(super) enum IpcAction {
    StreamSetup,
    HealthCheck,
    Forward,
    Job,
    Reload,
    Lifecycle,
    Unexpected,
}
