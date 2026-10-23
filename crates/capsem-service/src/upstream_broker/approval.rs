//! Approval descriptors inherit the registered worker's identity and authority.
use super::*;
use capsem_core::security_engine::approval::ApprovalBinding;

pub(super) struct Source {
    state: std::sync::Weak<crate::ServiceState>,
    binding: ApprovalBinding,
    directory: PathBuf,
    opened: bool,
}

impl PendingBroker {
    pub(crate) fn with_approvals(
        mut self,
        state: &Arc<crate::ServiceState>,
        id: &str,
        generation: uuid::Uuid,
        directory: &Path,
    ) -> Self {
        self.approval = Some(Source {
            state: Arc::downgrade(state),
            binding: ApprovalBinding {
                session_id: id.into(),
                generation,
            },
            directory: directory.into(),
            opened: false,
        });
        self
    }
}

pub(super) async fn open(
    source: &mut Option<Source>,
    authority: WorkerGrant,
    policy: tokio::sync::watch::Receiver<Arc<BrokerPolicy>>,
    responses: &WireSender,
    request_id: u64,
    tasks: &mut tokio::task::JoinSet<Result<(), String>>,
) -> Result<(), String> {
    let Some(source) = source.as_mut() else {
        return send_denied(responses, request_id, UpstreamGrantDenial::NotConfigured).await;
    };
    if source.opened || authority.is_revoked() {
        return send_denied(responses, request_id, UpstreamGrantDenial::InvalidResource).await;
    }
    let state = source.state.upgrade().ok_or("approval coordinator stopped")?;
    let current = state
        .instances
        .lock()
        .unwrap()
        .get(&source.binding.session_id)
        .is_some_and(|instance| {
            instance.generation == source.binding.generation && !instance.authority.grant().is_revoked()
        });
    if !current {
        return send_denied(responses, request_id, UpstreamGrantDenial::Revoked).await;
    }
    let (worker, server) = state
        .approvals
        .open(&state, source.binding.clone(), &source.directory, authority, policy)
        .await?;
    source.opened = true;
    let descriptor = OwnedFd::from(worker);
    tasks.spawn(server);
    send_response(
        responses,
        &UpstreamGrantResponse::ApprovalGranted { request_id },
        Some(&descriptor),
    )
    .await
}
