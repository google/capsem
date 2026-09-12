//! The guest end of this VM's private link.
//!
//! The guest's `capsem-tun` connects once at boot on VSOCK 5009 and again
//! whenever its stream ends. This owner holds the current connection and
//! nothing else: frames are switched by the network's confined process, which
//! receives a duplicate of this descriptor when the service links the VM.
use capsem_core::VsockConnection;
use std::sync::Mutex;
use tokio::sync::Notify;

pub(crate) struct PrivateLink {
    guest: Mutex<Option<VsockConnection>>,
    /// Woken whenever a guest stream arrives, for a link waiting on one.
    arrived: Notify,
}

impl PrivateLink {
    pub(crate) fn new() -> Self {
        Self {
            guest: Mutex::new(None),
            arrived: Notify::new(),
        }
    }

    /// The guest connected (again): this stream is the link from now on. The
    /// previous one, if any, ends here, which the framework object's release
    /// makes final for the guest.
    pub(crate) fn attach_guest(&self, conn: VsockConnection) {
        let previous = self.guest.lock().unwrap().replace(conn);
        drop(previous);
        self.arrived.notify_waiters();
    }
}

#[cfg(test)]
mod tests;
