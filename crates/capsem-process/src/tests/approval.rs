//! Real inherited descriptor setup shared by process approval boundary tests.
use capsem_core::net::upstream_grant::UpstreamGrantClient;
use capsem_core::security_engine::approval::client::{frame_limit, ApprovalCapability};
use capsem_foundation::ipc_channel::{self, Receiver, Sender};
use capsem_foundation::unix::router_channel::{DescriptorReceiver, DescriptorSender};
use capsem_proto::approval::{ApprovalClientMessage, ApprovalServerMessage};
use capsem_proto::upstream_grant::*;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::sync::Arc;

pub(crate) struct ApprovalPeer {
    pub(crate) capability: Arc<ApprovalCapability>,
    pub(crate) sender: Sender<ApprovalServerMessage>,
    pub(crate) receiver: Receiver<ApprovalClientMessage>,
    _descriptor: UnixStream,
}

impl ApprovalPeer {
    pub(crate) async fn new() -> Self {
        let (broker, worker) = UnixStream::pair().unwrap();
        let capability = Arc::new(ApprovalCapability::new(Arc::new(
            UpstreamGrantClient::start(worker).unwrap(),
        )));
        let requests =
            DescriptorReceiver::<UPSTREAM_GRANT_FRAME_SIZE, UPSTREAM_GRANT_MAX_FDS>::new(broker.try_clone().unwrap())
                .unwrap();
        let responses = DescriptorSender::<UPSTREAM_GRANT_FRAME_SIZE, UPSTREAM_GRANT_MAX_FDS>::new(broker).unwrap();
        let request = requests.recv().await.unwrap();
        let UpstreamGrantRequest::OpenApproval { request_id } = decode_upstream_grant_request(&request.bytes).unwrap()
        else {
            panic!("approval grant")
        };
        let (descriptor, service) = UnixStream::pair().unwrap();
        responses
            .send(
                &encode_upstream_grant_response(&UpstreamGrantResponse::ApprovalGranted { request_id }).unwrap(),
                &[descriptor.as_raw_fd()],
            )
            .await
            .unwrap();
        let (sender, receiver) = ipc_channel::channel_from_std_with_limit(service, frame_limit()).unwrap();
        Self {
            capability,
            sender,
            receiver,
            _descriptor: descriptor,
        }
    }
}
