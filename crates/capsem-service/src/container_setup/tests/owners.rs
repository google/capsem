//! Shared image-setup owner fixture and its controller contract.
use super::*;

pub(super) fn setup_message_count() -> usize {
    // Pull admission, options import, every controller import, then exec.
    capsem_core::container::LAUNCHER_FILES.len() + 3
}

pub(super) fn control_file_names() -> Vec<&'static str> {
    std::iter::once("options.json")
        .chain(capsem_core::container::LAUNCHER_FILES.iter().map(|(name, _)| *name))
        .collect()
}

pub(super) fn owner_accepting_stage_and_launch(uds_path: &StdPath) -> tokio::task::JoinHandle<Vec<ServiceToProcess>> {
    spawn_fake_process(uds_path, setup_message_count(), |message| {
        let reply = match message {
            ServiceToProcess::AdmitContainerPull { id, .. } => Some(ProcessToService::ContainerPullAdmission {
                id: *id,
                error: None,
                policy_refused: false,
            }),
            ServiceToProcess::LogFileBoundary { id, .. } => Some(ProcessToService::LogFileBoundaryResult {
                id: *id,
                success: true,
                data: None,
                error: None,
            }),
            ServiceToProcess::Exec { id, .. } => Some(ProcessToService::ExecResult {
                id: *id,
                stdout: vec![],
                stderr: vec![],
                exit_code: 0,
                truncated: false,
            }),
            other => panic!("unexpected IPC message during container setup: {other:?}"),
        };
        Box::pin(async move { reply })
    })
}

pub(super) fn owner_admitting_pull(uds_path: &StdPath) -> tokio::task::JoinHandle<Vec<ServiceToProcess>> {
    spawn_fake_process(uds_path, 1, |message| {
        let reply = match message {
            ServiceToProcess::AdmitContainerPull { id, .. } => Some(ProcessToService::ContainerPullAdmission {
                id: *id,
                error: None,
                policy_refused: false,
            }),
            other => panic!("pull admission sent an unexpected owner message: {other:?}"),
        };
        Box::pin(async move { reply })
    })
}
