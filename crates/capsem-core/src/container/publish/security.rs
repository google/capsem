//! Trusted publication facts and control leases stay in the VM owner.
use super::*;
use crate::security_engine::network::{ledger::NetworkSecurity, *};
use crate::security_engine::{RuntimeSecurityEventType, SecurityEnforcementAction, SecurityEvent};
use std::net::SocketAddr;

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
                generation: self.generation,
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

    /// Record that an exposure was closed. The close has already happened
    /// whatever the rules say: refusing to stop exposing a port protects
    /// nothing. An audit that cannot be admitted is reported to the caller.
    pub(super) async fn audit_revoked(
        &self,
        publication_id: uuid::Uuid,
        listener: SocketAddr,
        guest_port: u16,
        target: capsem_proto::PublicationTarget,
        access: capsem_proto::PublicationAccess,
    ) -> Result<()> {
        let authority = self.security.clone().context("publication security context missing")?;
        authority
            .evaluate_exposure(
                publication_id,
                listener,
                guest_port,
                target,
                access,
                NetworkLifecycleAction::Revoked,
            )
            .await
            .map(drop)
    }

    /// Recheck immediately before writing ConnectPort on the current control
    /// stream. A reconnect must never replay an old queued setup request.
    pub fn pending_connection(&self, flow: capsem_proto::router::FlowKey) -> bool {
        flow.generation == self.generation.get()
            && self.pending.lock().unwrap().get(&flow.id).is_some_and(|entry| {
                entry.data.is_some() && entry.lease.as_ref().is_some_and(|lease| !lease.is_cancelled())
            })
    }

    /// Admit a service-owned registry pull before the service constructs a
    /// registry client. Credentials never enter this owner request or event.
    pub async fn admit_container_pull(&self, image: String, registry: String, digest: Option<String>) -> Result<()> {
        let authority = self
            .security
            .clone()
            .context("container pull security context missing")?;
        let event = SecurityEvent::new(RuntimeSecurityEventType::NetworkLifecycle)
            .with_network(NetworkSecurityEvent::ContainerPull {
                vm: authority.vm.clone(),
            })
            .with_container(crate::security_engine::ContainerSecurityEvent {
                image: image.clone(),
                registry,
                digest,
            });
        let decision = tokio::time::timeout(Duration::from_secs(2), authority.engine.evaluate_and_record(event))
            .await
            .context("container pull audit deadline exceeded")??;
        match decision.action {
            SecurityEnforcementAction::Allow => Ok(()),
            SecurityEnforcementAction::Ask => Err(ContainerPullRefused(format!(
                "pulling container image {image} needs approval, which container setup cannot ask for"
            ))
            .into()),
            SecurityEnforcementAction::Block => Err(ContainerPullRefused(format!(
                "pulling container image {image} is blocked by policy{}",
                decision.reason.map(|reason| format!(": {reason}")).unwrap_or_default()
            ))
            .into()),
        }
    }
}

impl Authority {
    async fn evaluate_exposure(
        &self,
        publication_id: uuid::Uuid,
        listener: SocketAddr,
        guest_port: u16,
        target: capsem_proto::PublicationTarget,
        access: capsem_proto::PublicationAccess,
        action: NetworkLifecycleAction,
    ) -> Result<crate::security_engine::SecurityEnforcementDecision> {
        let event = SecurityEvent::new(RuntimeSecurityEventType::NetworkLifecycle).with_network(
            NetworkSecurityEvent::Exposure(NetworkExposure {
                publication_id,
                target,
                access,
                action,
                listener,
                destination: NetworkEndpoint {
                    vm: Some(self.vm.clone()),
                    address: (Ipv4Addr::LOCALHOST, guest_port).into(),
                },
            }),
        );
        Ok(
            tokio::time::timeout(Duration::from_secs(2), self.engine.evaluate_and_record(event))
                .await
                .context("exposure audit deadline exceeded")??,
        )
    }

    /// Evaluate an exposure opening against the VM's current rules and
    /// plugins, with its audit row admitted first. Anything but allow refuses
    /// it: an exposure change has no one to ask.
    pub(super) async fn admit_exposure(
        &self,
        publication_id: uuid::Uuid,
        listener: SocketAddr,
        guest_port: u16,
        target: capsem_proto::PublicationTarget,
        access: capsem_proto::PublicationAccess,
        action: NetworkLifecycleAction,
    ) -> Result<()> {
        let decision = self
            .evaluate_exposure(publication_id, listener, guest_port, target, access, action)
            .await?;
        match decision.action {
            SecurityEnforcementAction::Allow => Ok(()),
            SecurityEnforcementAction::Ask => Err(ExposureRefused(format!(
                "exposure of guest port {guest_port} needs approval, which exposure changes cannot ask for"
            ))
            .into()),
            SecurityEnforcementAction::Block => Err(ExposureRefused(format!(
                "exposure of guest port {guest_port} is blocked by policy{}",
                decision.reason.map(|reason| format!(": {reason}")).unwrap_or_default()
            ))
            .into()),
        }
    }
}

/// The VM's rules refused an exposure, as opposed to it failing to open.
#[derive(Debug)]
pub struct ExposureRefused(pub String);

impl std::fmt::Display for ExposureRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ExposureRefused {}

/// The VM's effective policy refused a service-owned container image pull.
#[derive(Debug)]
pub struct ContainerPullRefused(pub String);

impl std::fmt::Display for ContainerPullRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ContainerPullRefused {}

#[derive(Clone)]
pub struct AuditFlow {
    authority: Arc<Authority>,
    facts: NetworkFlow,
    started: std::time::Instant,
}

impl AuditFlow {
    pub(super) fn new(
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

    pub(super) fn preview(
        authority: Arc<Authority>,
        publication_id: uuid::Uuid,
        listener: SocketAddr,
        peer: SocketAddr,
        port: u16,
        kind: capsem_proto::PreviewAdmissionKind,
    ) -> Self {
        Self {
            facts: NetworkFlow {
                connection_id: uuid::Uuid::new_v4(),
                route: NetworkRoute::Preview {
                    publication_id,
                    listener,
                    kind,
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

    /// This VM's link to a network's switch: its own address on both ends,
    /// no ports, judged once when the service links it.
    pub(super) fn link(authority: Arc<Authority>, network: NetworkIdentity, own: Ipv4Addr) -> Self {
        let endpoint = || NetworkEndpoint {
            vm: Some(authority.vm.clone()),
            address: (own, 0).into(),
        };
        Self {
            facts: NetworkFlow {
                connection_id: uuid::Uuid::new_v4(),
                route: NetworkRoute::Private { network },
                side: NetworkSide::Destination,
                protocol: NetworkProtocol::Link,
                source: endpoint(),
                destination: endpoint(),
                report: None,
            },
            authority,
            started: std::time::Instant::now(),
        }
    }

    pub fn facts(&self) -> &NetworkFlow {
        &self.facts
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
