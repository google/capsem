//! Kernel identity attached to every request accepted on the service UDS.

use axum::extract::connect_info::Connected;
use axum::serve::IncomingStream;
use std::os::fd::AsFd;
use tokio::net::UnixListener;

#[derive(Clone, Copy, Debug)]
pub(crate) struct ServicePeer(pub(crate) Option<capsem_foundation::unix::peer::PeerIdentity>);

impl Connected<IncomingStream<'_, UnixListener>> for ServicePeer {
    fn connect_info(stream: IncomingStream<'_, UnixListener>) -> Self {
        Self(capsem_foundation::unix::peer::identity(stream.io().as_fd()).ok())
    }
}
