use std::sync::Arc;

use capsem_proto::ipc::{ProcessToService, ServiceToProcess};
use tokio::sync::mpsc::Sender;

use crate::job_store::JobStore;

pub(super) fn spawn(jobs: &Arc<JobStore>, output: &Sender<ProcessToService>, request: ServiceToProcess) {
    let ServiceToProcess::AdmitContainerPull {
        id,
        image,
        registry,
        digest,
    } = request
    else {
        unreachable!("container pull handler received another request")
    };
    let jobs = Arc::clone(jobs);
    let output = output.clone();
    tokio::spawn(async move {
        let response = match jobs.publisher.admit_container_pull(image, registry, digest).await {
            Ok(()) => ProcessToService::ContainerPullAdmission {
                id,
                error: None,
                policy_refused: false,
            },
            Err(error) => ProcessToService::ContainerPullAdmission {
                id,
                policy_refused: error.is::<capsem_core::container::publish::ContainerPullRefused>(),
                error: Some(format!("{error:#}")),
            },
        };
        capsem_core::try_send!("container_pull_admission", output.send(response).await);
    });
}
