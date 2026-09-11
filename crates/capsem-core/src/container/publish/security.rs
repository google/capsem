//! Trusted publication facts and control leases stay in the VM owner.
use super::*;
use crate::security_engine::network::{ledger::NetworkSecurity, *};
use crate::security_engine::{RuntimeSecurityEventType, SecurityEnforcementAction, SecurityEvent};
use std::net::SocketAddr;
use std::num::NonZeroU64;

pub(super) struct Authority {
    vm: NetworkVm,
    engine: Arc<NetworkSecurity>,
}

impl Publisher {
    pub fn with_security(mut self, id: String, name: String, engine: Arc<NetworkSecurity>) -> Self {
        self.security = Some(Arc::new(Authority {
            vm: NetworkVm {
                id,
                name,
                generation: NonZeroU64::new(self.generation).expect("nonzero boot generation"),
            },
            engine,
        }));
        self
    }

    /// Called only by the host control actor after successful replay.
    pub fn control_ready(&self) -> Result<()> {
        let mut lease = self.control_lease.lock().unwrap();
        ensure!(lease.is_none(), "guest control lease is already live");
        ensure!(!self.cancellation.is_cancelled(), "VM router is shutting down");
        *lease = Some(self.cancellation.child_token());
        drop(lease);
        Ok(())
    }

    /// Recheck immediately before writing ConnectPort on the current control
    /// stream. A reconnect must never replay an old queued setup request.
    pub fn pending_connection(&self, flow: capsem_proto::router::FlowKey) -> bool {
        flow.generation == self.generation
            && self.pending.lock().unwrap().get(&flow.id).is_some_and(|entry| {
                entry.data.is_some() && entry.lease.as_ref().is_some_and(|lease| !lease.is_cancelled())
            })
    }
}

#[derive(Clone)]
pub(super) struct AuditFlow {
    authority: Arc<Authority>,
    facts: NetworkFlow,
    started: std::time::Instant,
}

impl AuditFlow {
    pub fn new(
        authority: Arc<Authority>,
        publication_id: uuid::Uuid,
        listener: SocketAddr,
        peer: SocketAddr,
        port: u16,
    ) -> Self {
        Self {
            facts: NetworkFlow {
                connection_id: uuid::Uuid::new_v4(),
                route: NetworkRoute::Expose {
                    publication_id,
                    listener,
                },
                side: NetworkSide::Destination,
                protocol: NetworkProtocol::Tcp,
                source: NetworkEndpoint {
                    vm: None,
                    address: peer,
                },
                destination: NetworkEndpoint {
                    vm: Some(authority.vm.clone()),
                    address: (Ipv4Addr::LOCALHOST, port).into(),
                },
                report: None,
            },
            authority,
            started: std::time::Instant::now(),
        }
    }

    pub async fn authorize(&self) -> Result<SecurityEnforcementAction> {
        let event = SecurityEvent::new(RuntimeSecurityEventType::NetworkConnect)
            .with_network(NetworkSecurityEvent::Flow(self.facts.clone()));
        Ok(self.authority.engine.evaluate_and_record(event).await?.action)
    }

    pub async fn record(
        &self,
        kind: RuntimeSecurityEventType,
        reason: NetworkReason,
        sent: u64,
        received: u64,
    ) -> Result<()> {
        let mut facts = self.facts.clone();
        facts.report = Some(NetworkReport {
            reason,
            elapsed_ms: self.started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
            bytes_sent: sent,
            bytes_received: received,
        });
        let event = SecurityEvent::new(kind).with_network(NetworkSecurityEvent::Flow(facts));
        tokio::time::timeout(Duration::from_secs(2), self.authority.engine.evaluate_and_record(event))
            .await
            .context("publication audit deadline exceeded")??;
        Ok(())
    }
}

pub(super) fn close_reason(reason: capsem_proto::router::CloseReason) -> NetworkReason {
    use capsem_proto::router::CloseReason as Reason;
    match reason {
        Reason::Complete => NetworkReason::Complete,
        Reason::Reset => NetworkReason::Reset,
        Reason::WriteStall => NetworkReason::WriteStall,
        Reason::HalfCloseTimeout => NetworkReason::HalfCloseTimeout,
        Reason::Cancelled => NetworkReason::Cancelled,
        Reason::Io => NetworkReason::Io,
    }
}
