//! Trusted routing facts and audit-only observations on the existing security rail.
//! These types are constructed by VM owners, never deserialized from guest requests.
use super::{PolicySubjectValue, RuntimeSecurityEventFamily, RuntimeSecurityEventType, SecurityActionError};
use serde::Serialize;
use std::borrow::Cow;
use std::net::SocketAddr;
use std::num::NonZeroU64;
use uuid::Uuid;

pub mod ledger;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NetworkIdentity {
    pub id: Uuid,
    pub name: String,
}

impl NetworkIdentity {
    /// From the wire, where the id travels as text.
    pub fn parse(id: &str, name: String) -> Result<Self, String> {
        Ok(Self {
            id: Uuid::parse_str(id).map_err(|error| format!("network id {id:?}: {error}"))?,
            name,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NetworkVm {
    pub id: String,
    pub name: String,
    /// Decimal string in JSON: boot identity must survive JavaScript's number range.
    #[serde(serialize_with = "serialize_generation")]
    pub generation: NonZeroU64,
}

fn serialize_generation<S: serde::Serializer>(generation: &NonZeroU64, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.collect_str(generation)
}

impl super::SecurityEvent {
    pub(super) fn validate_network(&self, kind: RuntimeSecurityEventType) -> Result<(), SecurityActionError> {
        if kind.family() == RuntimeSecurityEventFamily::Network
            || self.event_type.family() == RuntimeSecurityEventFamily::Network
        {
            if kind != self.event_type {
                return Err(SecurityActionError::new(
                    "network event: inconsistent ledger event type",
                ));
            }
            self.network
                .as_ref()
                .ok_or_else(|| SecurityActionError::new("network event: missing routing facts"))?
                .validate(kind)?;
        }
        Ok(())
    }

    pub fn with_network(mut self, network: NetworkSecurityEvent) -> Self {
        self.network = Some(network);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NetworkEndpoint {
    pub vm: Option<NetworkVm>,
    pub address: SocketAddr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkSide {
    Source,
    Destination,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkProtocol {
    Tcp,
    /// The VM's frame link to a network's switch, evaluated once per attach:
    /// every UDP and ICMP packet between members rides it.
    Link,
    SyntheticPing,
}

impl NetworkProtocol {
    /// Whether the endpoints of this protocol carry no port.
    pub fn portless(self) -> bool {
        matches!(self, Self::Link | Self::SyntheticPing)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum NetworkRoute {
    Expose {
        publication_id: Uuid,
        listener: SocketAddr,
    },
    Preview {
        publication_id: Uuid,
        listener: SocketAddr,
        kind: capsem_proto::PreviewAdmissionKind,
    },
    Private {
        network: NetworkIdentity,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkReason {
    Connected,
    Blocked,
    ApprovalRequired,
    Refused,
    StaleGeneration,
    Quota,
    Complete,
    Reset,
    WriteStall,
    HalfCloseTimeout,
    SetupTimeout,
    Cancelled,
    Io,
    Unreachable,
}

/// Never exposed through CEL: child reports cannot authorize a connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NetworkReport {
    pub reason: NetworkReason,
    pub elapsed_ms: u64,
    pub bytes_sent: u64,
    pub bytes_received: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NetworkFlow {
    pub connection_id: Uuid,
    pub route: NetworkRoute,
    pub side: NetworkSide,
    pub protocol: NetworkProtocol,
    pub source: NetworkEndpoint,
    pub destination: NetworkEndpoint,
    pub report: Option<NetworkReport>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkLifecycleAction {
    Created,
    Connected,
    Disconnected,
    Stopped,
    Resumed,
    Retired,
    Deleted,
    /// An exposure's loopback listener was opened on request.
    Published,
    /// A saved exposure was reopened when its VM owner started again.
    Restored,
    /// An exposure was closed for good.
    Revoked,
}

impl NetworkLifecycleAction {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Connected => "connected",
            Self::Disconnected => "disconnected",
            Self::Stopped => "stopped",
            Self::Resumed => "resumed",
            Self::Retired => "retired",
            Self::Deleted => "deleted",
            Self::Published => "published",
            Self::Restored => "restored",
            Self::Revoked => "revoked",
        }
    }

    const fn is_exposure(self) -> bool {
        matches!(self, Self::Published | Self::Restored | Self::Revoked)
    }
}

/// A loopback exposure opening or closing on its VM owner: the listener the
/// host reaches and the guest endpoint its connections will be sent to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NetworkExposure {
    pub publication_id: Uuid,
    pub target: capsem_proto::PublicationTarget,
    pub access: capsem_proto::PublicationAccess,
    pub action: NetworkLifecycleAction,
    pub listener: SocketAddr,
    pub destination: NetworkEndpoint,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "context", rename_all = "snake_case")]
pub enum NetworkSecurityEvent {
    Flow(NetworkFlow),
    Lifecycle {
        network: NetworkIdentity,
        vm: Option<NetworkVm>,
        action: NetworkLifecycleAction,
    },
    Exposure(NetworkExposure),
    ContainerPull {
        vm: NetworkVm,
    },
}

impl NetworkSecurityEvent {
    pub(super) fn validate(&self, kind: RuntimeSecurityEventType) -> Result<(), SecurityActionError> {
        use RuntimeSecurityEventType as Type;
        match self {
            Self::Lifecycle {
                network, vm, action, ..
            } => {
                require(kind == Type::NetworkLifecycle, "lifecycle context on a flow event")?;
                require(!action.is_exposure(), "exposure action on a network lifecycle")?;
                validate_network(network)?;
                if let Some(vm) = vm {
                    validate_vm(vm)?;
                }
            }
            Self::Exposure(exposure) => {
                require(kind == Type::NetworkLifecycle, "exposure context on a flow event")?;
                require(exposure.action.is_exposure(), "network action on an exposure")?;
                require(!exposure.publication_id.is_nil(), "missing publication identity")?;
                require(
                    exposure.listener.ip().is_loopback() && exposure.listener.port() != 0,
                    "invalid publication listener",
                )?;
                let vm = exposure
                    .destination
                    .vm
                    .as_ref()
                    .ok_or_else(|| SecurityActionError::new("network event: missing destination VM identity"))?;
                validate_vm(vm)?;
                require(
                    exposure.destination.address.ip().is_loopback()
                        && exposure.target.admits(exposure.destination.address.port()),
                    "invalid exposure destination",
                )?;
            }
            Self::ContainerPull { vm } => {
                require(kind == Type::NetworkLifecycle, "container pull context on a flow event")?;
                validate_vm(vm)?;
            }
            Self::Flow(flow) => {
                require(!flow.connection_id.is_nil(), "missing connection identity")?;
                require(kind != Type::NetworkLifecycle, "flow context on a lifecycle event")?;
                let probe = matches!(kind, Type::NetworkProbe | Type::NetworkProbeResult);
                require(
                    probe == (flow.protocol == NetworkProtocol::SyntheticPing),
                    "protocol/event mismatch",
                )?;
                let request = matches!(kind, Type::NetworkConnect | Type::NetworkProbe);
                require(request == flow.report.is_none(), "request/result report mismatch")?;
                for endpoint in [&flow.source, &flow.destination] {
                    if let Some(vm) = &endpoint.vm {
                        validate_vm(vm)?;
                    }
                    require(!endpoint.address.ip().is_unspecified(), "unspecified endpoint address")?;
                    require(
                        flow.protocol.portless() == (endpoint.address.port() == 0),
                        "invalid endpoint port",
                    )?;
                }
                require(flow.destination.vm.is_some(), "missing destination VM identity")?;
                match &flow.route {
                    NetworkRoute::Expose {
                        publication_id,
                        listener,
                    }
                    | NetworkRoute::Preview {
                        publication_id,
                        listener,
                        ..
                    } => {
                        require(
                            listener.ip().is_loopback() && listener.port() != 0,
                            "invalid publication listener",
                        )?;
                        require(!publication_id.is_nil(), "missing publication identity")?;
                        require(!probe, "expose cannot carry synthetic ping")?;
                        require(flow.source.vm.is_none(), "expose source must be a host socket")?;
                        require(
                            flow.side == NetworkSide::Destination,
                            "expose checks its destination owner",
                        )?;
                    }
                    NetworkRoute::Private { network } => {
                        validate_network(network)?;
                        require(flow.source.vm.is_some(), "missing source VM identity")?;
                        require(
                            flow.source.address.is_ipv4() && flow.destination.address.is_ipv4(),
                            "private IPv4 required",
                        )?;
                    }
                }
            }
        }
        Ok(())
    }

    pub(super) fn get(&self, field: &str) -> Option<PolicySubjectValue<'_>> {
        if field == "valid" {
            return Some(PolicySubjectValue::Bool(true));
        }
        if let Self::Exposure(exposure) = self {
            return exposure_field(exposure, field);
        }
        if let Self::ContainerPull { vm } = self {
            return match field {
                "mode" => Some(borrowed("registry_pull")),
                "action" => Some(borrowed("pull")),
                _ => field.strip_prefix("destination.").and_then(|field| vm_field(vm, field)),
            };
        }
        let network = match self {
            Self::Lifecycle { network, .. } => Some(network),
            Self::Flow(NetworkFlow {
                route: NetworkRoute::Private { network },
                ..
            }) => Some(network),
            Self::Flow(_) | Self::Exposure(_) | Self::ContainerPull { .. } => None,
        };
        if let (Self::Lifecycle { action, .. }, "action") = (self, field) {
            return Some(borrowed(action.as_str()));
        }
        match field {
            "id" => return network.map(|network| owned(network.id)),
            "name" => return network.map(|network| borrowed(&network.name)),
            _ => {}
        }
        let Self::Flow(flow) = self else { return None };
        match field {
            "mode" => Some(borrowed(match flow.route {
                NetworkRoute::Expose { .. } => "expose",
                NetworkRoute::Preview { .. } => "http_preview",
                NetworkRoute::Private { .. } => "private",
            })),
            "side" => Some(borrowed(match flow.side {
                NetworkSide::Source => "source",
                NetworkSide::Destination => "destination",
            })),
            "protocol" => Some(borrowed(match flow.protocol {
                NetworkProtocol::Tcp => "tcp",
                NetworkProtocol::Link => "link",
                NetworkProtocol::SyntheticPing => "synthetic_ping",
            })),
            "publication.id" => match flow.route {
                NetworkRoute::Expose { publication_id, .. } | NetworkRoute::Preview { publication_id, .. } => {
                    Some(owned(publication_id))
                }
                _ => None,
            },
            "action" => match flow.route {
                NetworkRoute::Preview {
                    kind: capsem_proto::PreviewAdmissionKind::Request,
                    ..
                } => Some(borrowed("preview_request")),
                NetworkRoute::Preview {
                    kind: capsem_proto::PreviewAdmissionKind::WebsocketUpgrade,
                    ..
                } => Some(borrowed("preview_upgrade")),
                _ => None,
            },
            _ => field
                .strip_prefix("source.")
                .and_then(|field| endpoint_field(&flow.source, field))
                .or_else(|| {
                    field
                        .strip_prefix("destination.")
                        .and_then(|field| endpoint_field(&flow.destination, field))
                }),
        }
    }
}

/// An exposure's rule-visible facts: its mode, action, namespace and
/// publication, the loopback listener as its source, and the guest endpoint.
/// It has no side or protocol, so connection rules never match it.
fn exposure_field<'a>(exposure: &'a NetworkExposure, field: &str) -> Option<PolicySubjectValue<'a>> {
    match field {
        "mode" => Some(borrowed(match exposure.access {
            capsem_proto::PublicationAccess::LoopbackTcp => "expose",
            capsem_proto::PublicationAccess::HttpPreview => "http_preview",
        })),
        "action" => Some(borrowed(exposure.action.as_str())),
        "target" => Some(borrowed(match exposure.target {
            capsem_proto::PublicationTarget::Container => "container",
            capsem_proto::PublicationTarget::Vm => "vm",
        })),
        "publication.id" => Some(owned(exposure.publication_id)),
        "source.ip" => Some(owned(exposure.listener.ip())),
        "source.port" => Some(owned(exposure.listener.port())),
        _ => field
            .strip_prefix("destination.")
            .and_then(|field| endpoint_field(&exposure.destination, field)),
    }
}

fn endpoint_field<'a>(endpoint: &'a NetworkEndpoint, field: &str) -> Option<PolicySubjectValue<'a>> {
    match field {
        "vm_id" => endpoint.vm.as_ref().map(|vm| borrowed(&vm.id)),
        "vm_name" => endpoint.vm.as_ref().map(|vm| borrowed(&vm.name)),
        "generation" => endpoint.vm.as_ref().map(|vm| owned(vm.generation)),
        "ip" => Some(owned(endpoint.address.ip())),
        "port" => Some(owned(endpoint.address.port())),
        _ => None,
    }
}

fn vm_field<'a>(vm: &'a NetworkVm, field: &str) -> Option<PolicySubjectValue<'a>> {
    match field {
        "vm_id" => Some(borrowed(&vm.id)),
        "vm_name" => Some(borrowed(&vm.name)),
        "generation" => Some(owned(vm.generation)),
        _ => None,
    }
}

fn validate_network(network: &NetworkIdentity) -> Result<(), SecurityActionError> {
    require(!network.id.is_nil(), "missing network identity")?;
    validate_label(&network.name)
}

fn validate_vm(vm: &NetworkVm) -> Result<(), SecurityActionError> {
    validate_label(&vm.id)?;
    validate_label(&vm.name)
}

fn validate_label(value: &str) -> Result<(), SecurityActionError> {
    require(
        !value.is_empty()
            && value.len() <= 253
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte)),
        "invalid routing identity",
    )
}

fn require(valid: bool, reason: &str) -> Result<(), SecurityActionError> {
    if valid {
        Ok(())
    } else {
        Err(SecurityActionError::new(format!("network event: {reason}")))
    }
}

fn borrowed(value: &str) -> PolicySubjectValue<'_> {
    PolicySubjectValue::String(Cow::Borrowed(value))
}

fn owned(value: impl ToString) -> PolicySubjectValue<'static> {
    PolicySubjectValue::String(Cow::Owned(value.to_string()))
}

#[cfg(test)]
pub(super) mod tests;

impl From<capsem_logger::TransportEventKind> for RuntimeSecurityEventType {
    fn from(kind: capsem_logger::TransportEventKind) -> Self {
        use capsem_logger::TransportEventKind as Kind;
        match kind {
            Kind::Connect => Self::NetworkConnect,
            Kind::ConnectResult => Self::NetworkConnectResult,
            Kind::Close => Self::NetworkClose,
            Kind::Lifecycle => Self::NetworkLifecycle,
            Kind::Probe => Self::NetworkProbe,
            Kind::ProbeResult => Self::NetworkProbeResult,
        }
    }
}
