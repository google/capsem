//! Obtain the same inherited descriptor capability a confined proxy receives.
use super::*;
use capsem_core::security_engine::approval::client::{frame_limit, ApprovalCapability};
use capsem_foundation::ipc_channel;
use capsem_foundation::unix::router_channel::{DescriptorReceiver, DescriptorSender};
use capsem_proto::approval::{ApprovalClientMessage, ApprovalServerMessage};
use capsem_proto::upstream_grant::{
    decode_upstream_grant_request, encode_upstream_grant_response, UpstreamGrantRequest, UpstreamGrantResponse,
    UPSTREAM_GRANT_FRAME_SIZE, UPSTREAM_GRANT_MAX_FDS,
};
use std::os::fd::AsRawFd;

pub(super) struct ApprovalFixture {
    pub capability: Arc<ApprovalCapability>,
    pub sender: ipc_channel::Sender<ApprovalServerMessage>,
    pub receiver: ipc_channel::Receiver<ApprovalClientMessage>,
    keep: tokio::sync::oneshot::Sender<()>,
    worker: tokio::task::JoinHandle<()>,
}

impl ApprovalFixture {
    pub fn new() -> Self {
        let (worker, coordinator) = std::os::unix::net::UnixStream::pair().unwrap();
        let (grant_worker, grant_broker) = std::os::unix::net::UnixStream::pair().unwrap();
        let grant_client = capsem_core::net::upstream_grant::UpstreamGrantClient::start(grant_worker).unwrap();
        let grants = DescriptorReceiver::<UPSTREAM_GRANT_FRAME_SIZE, UPSTREAM_GRANT_MAX_FDS>::new(
            grant_broker.try_clone().unwrap(),
        )
        .unwrap();
        let responses =
            DescriptorSender::<UPSTREAM_GRANT_FRAME_SIZE, UPSTREAM_GRANT_MAX_FDS>::new(grant_broker).unwrap();
        let (keep, finish) = tokio::sync::oneshot::channel();
        let grant_task = tokio::spawn(async move {
            let request = grants.recv().await.unwrap();
            assert!(request.fds.is_empty());
            let UpstreamGrantRequest::OpenApproval { request_id } =
                decode_upstream_grant_request(&request.bytes).unwrap()
            else {
                panic!("only the inherited approval capability may be opened")
            };
            responses
                .send(
                    &encode_upstream_grant_response(&UpstreamGrantResponse::ApprovalGranted { request_id }).unwrap(),
                    &[worker.as_raw_fd()],
                )
                .await
                .unwrap();
            let _ = finish.await;
        });
        let (sender, receiver) = ipc_channel::channel_from_std_with_limit(coordinator, frame_limit()).unwrap();
        Self {
            capability: Arc::new(ApprovalCapability::new(Arc::new(grant_client))),
            sender,
            receiver,
            keep,
            worker: grant_task,
        }
    }

    pub async fn finish(self) {
        self.keep.send(()).unwrap();
        self.worker.await.unwrap();
    }
}
