use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use capsem_core::net::mitm_proxy::{
    protocol::Protocol, GrantedTcpStream, TcpConnectGrantFuture, TcpGrantSelection, TcpResolveGrantFuture,
    TcpUpstreamGrants, UpstreamTarget,
};
use capsem_core::net::proxy_engine::ProxyPolicyHandle;
use capsem_core::net::upstream_address::UpstreamResolver;

pub(super) struct IntegrationGrants {
    policy: ProxyPolicyHandle,
    resolver: UpstreamResolver,
    selections: Arc<Mutex<HashMap<u64, UpstreamTarget>>>,
    next_id: AtomicU64,
}

impl IntegrationGrants {
    pub(super) fn new(policy: ProxyPolicyHandle, resolver: UpstreamResolver) -> Self {
        Self {
            policy,
            resolver,
            selections: Arc::new(Mutex::new(HashMap::new())),
            next_id: AtomicU64::new(1),
        }
    }
}

impl TcpUpstreamGrants for IntegrationGrants {
    fn resolve(&self, protocol: Protocol, host: &str, port: u16, _policy_digest: &str) -> TcpResolveGrantFuture<'_> {
        let host = host.to_owned();
        Box::pin(async move {
            let policy = self.policy.snapshot();
            let target = UpstreamTarget::resolve(&self.resolver, policy.network(), &host, port).await;
            if let UpstreamTarget::Unresolved(error) = &target {
                return Err(io::Error::new(io::ErrorKind::NotFound, error.clone()));
            }
            let judged_ip = target.judged_ip(&host);
            let protocol = target.protocol(protocol);
            let selection_id = self.next_id.fetch_add(1, Ordering::Relaxed);
            self.selections.lock().unwrap().insert(selection_id, target);
            let selections = Arc::clone(&self.selections);
            Ok(TcpGrantSelection::new(selection_id, protocol, judged_ip, move || {
                selections.lock().unwrap().remove(&selection_id);
            }))
        })
    }

    fn connect(&self, selection_id: u64, _policy_digest: &str) -> TcpConnectGrantFuture<'_> {
        Box::pin(async move {
            let target = self
                .selections
                .lock()
                .unwrap()
                .remove(&selection_id)
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "unknown integration selection"))?;
            let (stream, _pinned) = target.connect().await?;
            Ok(GrantedTcpStream::new(stream, || {}))
        })
    }
}
