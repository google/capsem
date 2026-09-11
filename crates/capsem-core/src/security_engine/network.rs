//! Trusted routing facts and audit-only observations on the existing security rail.
//! These types are constructed by VM owners, never deserialized from guest requests.
use super::{PolicySubjectValue, RuntimeSecurityEventFamily, RuntimeSecurityEventType, SecurityActionError};
use serde::Serialize;
use std::borrow::Cow;
use std::net::SocketAddr;
use std::num::NonZeroU64;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NetworkIdentity {
    pub id: Uuid,
    pub name: String,
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
    SyntheticPing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum NetworkRoute {
    Expose { publication_id: Uuid },
    Private { network: NetworkIdentity },
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
}

impl NetworkSecurityEvent {
    pub(super) fn validate(&self, kind: RuntimeSecurityEventType) -> Result<(), SecurityActionError> {
        use RuntimeSecurityEventType as Type;
        match self {
            Self::Lifecycle { network, vm, .. } => {
                require(kind == Type::NetworkLifecycle, "lifecycle context on a flow event")?;
                validate_network(network)?;
                if let Some(vm) = vm {
                    validate_vm(vm)?;
                }
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
                    require(probe == (endpoint.address.port() == 0), "invalid endpoint port")?;
                }
                require(flow.destination.vm.is_some(), "missing destination VM identity")?;
                match &flow.route {
                    NetworkRoute::Expose { publication_id } => {
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
        let network = match self {
            Self::Lifecycle { network, .. } => Some(network),
            Self::Flow(NetworkFlow {
                route: NetworkRoute::Private { network },
                ..
            }) => Some(network),
            Self::Flow(_) => None,
        };
        match field {
            "id" => return network.map(|network| owned(network.id)),
            "name" => return network.map(|network| borrowed(&network.name)),
            _ => {}
        }
        let Self::Flow(flow) = self else { return None };
        match field {
            "mode" => Some(borrowed(match flow.route {
                NetworkRoute::Expose { .. } => "expose",
                NetworkRoute::Private { .. } => "private",
            })),
            "side" => Some(borrowed(match flow.side {
                NetworkSide::Source => "source",
                NetworkSide::Destination => "destination",
            })),
            "protocol" => Some(borrowed(match flow.protocol {
                NetworkProtocol::Tcp => "tcp",
                NetworkProtocol::SyntheticPing => "synthetic_ping",
            })),
            "publication.id" => match flow.route {
                NetworkRoute::Expose { publication_id } => Some(owned(publication_id)),
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
