use crate::job_store::JobStore;
use capsem_proto::ipc::{ProcessToService, ServiceToProcess};
use capsem_proto::{PreviewAdmissionKind, PublicationTarget};
use std::sync::Arc;
use tokio::sync::mpsc;

#[allow(clippy::too_many_arguments)]
pub(super) fn spawn_declare(
    jobs: &Arc<JobStore>,
    control: &mpsc::Sender<ServiceToProcess>,
    output: &mpsc::Sender<ProcessToService>,
    id: u64,
    preview_listener: Option<u16>,
    host_port: u16,
    guest_port: u16,
    target: PublicationTarget,
) {
    let jobs = Arc::clone(jobs);
    let control = control.clone();
    let output = output.clone();
    tokio::spawn(async move {
        let result = match preview_listener {
            Some(listener_port) => {
                jobs.publisher
                    .declare_preview(listener_port, guest_port, target, control)
                    .await
            }
            None => {
                jobs.publisher
                    .publish_saved(host_port, guest_port, target, control)
                    .await
            }
        };
        let response = match result {
            Ok(publication) => ProcessToService::PortPublished {
                id,
                publication: Some(publication),
                error: None,
                policy_refused: false,
            },
            Err(error) => ProcessToService::PortPublished {
                id,
                publication: None,
                policy_refused: error.is::<capsem_core::container::publish::ExposureRefused>(),
                error: Some(format!("{error:#}")),
            },
        };
        capsem_core::try_send!("publication_result", output.send(response).await);
    });
}

pub(super) fn spawn_revoke(
    jobs: &Arc<JobStore>,
    output: &mpsc::Sender<ProcessToService>,
    id: u64,
    exposure_id: String,
) {
    let jobs = Arc::clone(jobs);
    let output = output.clone();
    tokio::spawn(async move {
        let response = match jobs.publisher.revoke(&exposure_id).await {
            Ok(revoked) => ProcessToService::ExposureRevoked {
                id,
                revoked,
                error: None,
            },
            Err(error) => ProcessToService::ExposureRevoked {
                id,
                revoked: false,
                error: Some(format!("{error:#}")),
            },
        };
        capsem_core::try_send!("publication_revoke_result", output.send(response).await);
    });
}

pub(super) async fn create_session(
    jobs: &JobStore,
    output: &mpsc::Sender<ProcessToService>,
    id: u64,
    exposure_id: &str,
) {
    let result = jobs.publisher.create_preview_session(exposure_id);
    capsem_core::try_send!(
        "preview_session_result",
        output
            .send(ProcessToService::PreviewSessionCreated {
                id,
                bootstrap_token: result.as_ref().ok().cloned(),
                expires_in_seconds: 30,
                error: result.err().map(|error| format!("{error:#}")),
            })
            .await
    );
}

pub(super) async fn exchange_bootstrap(
    jobs: &JobStore,
    output: &mpsc::Sender<ProcessToService>,
    id: u64,
    exposure_id: &str,
    bootstrap_token: &str,
) {
    let result = jobs.publisher.exchange_preview_bootstrap(exposure_id, bootstrap_token);
    capsem_core::try_send!(
        "preview_bootstrap_result",
        output
            .send(ProcessToService::PreviewBootstrapExchanged {
                id,
                session_token: result.as_ref().ok().cloned(),
                expires_in_seconds: capsem_proto::PREVIEW_SESSION_LIFETIME_SECS,
                error: result.err().map(|error| format!("{error:#}")),
            })
            .await
    );
}

pub(super) async fn admit(
    jobs: &JobStore,
    output: &mpsc::Sender<ProcessToService>,
    id: u64,
    exposure_id: &str,
    session_token: &str,
    kind: PreviewAdmissionKind,
) {
    let result = jobs
        .publisher
        .admit_preview_connection(exposure_id, session_token, kind);
    let generation = jobs.publisher.generation().get();
    let handoff_socket = jobs
        .cable_seat
        .get()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default();
    let response = match result {
        Ok(handoff_token) if !handoff_socket.is_empty() => ProcessToService::PreviewConnectionAdmitted {
            id,
            handoff_socket,
            handoff_token,
            owner_generation: generation,
            error: None,
            policy_refused: false,
        },
        Ok(_) => ProcessToService::PreviewConnectionAdmitted {
            id,
            handoff_socket: String::new(),
            handoff_token: 0,
            owner_generation: generation,
            error: Some("preview handoff seat unavailable".into()),
            policy_refused: false,
        },
        Err(error) => ProcessToService::PreviewConnectionAdmitted {
            id,
            handoff_socket: String::new(),
            handoff_token: 0,
            owner_generation: generation,
            error: Some(format!("{error:#}")),
            policy_refused: error.is::<capsem_core::container::publish::ExposureRefused>(),
        },
    };
    capsem_core::try_send!("preview_admission_result", output.send(response).await);
}

pub(super) async fn list(jobs: &JobStore, output: &mpsc::Sender<ProcessToService>, id: u64) {
    let response = ProcessToService::PublicationList {
        id,
        generation: jobs.publisher.generation().get(),
        publications: jobs.publisher.publications(),
    };
    capsem_core::try_send!("publication_list_result", output.send(response).await);
}
