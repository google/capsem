//! Fixed, versioned records for coordinator-minted upstream descriptors.
//!
//! A worker first asks the trusted coordinator to resolve a TCP target. The
//! response carries an opaque selection id, the address policy must judge and
//! the effective HTTP/TLS protocol. Only after policy admits that selection
//! does the worker ask the coordinator to connect it. DNS requests name a
//! configured upstream by index, never by worker-supplied address.

use std::net::IpAddr;

use anyhow::{bail, Result};

use crate::ledger::{LedgerChannelGrant, LedgerClientRole, LedgerGeneration};
use crate::mcp_worker::McpHttpRole;

mod guest_metadata;
mod request;
pub use request::{decode_upstream_grant_request, encode_upstream_grant_request};

pub use guest_metadata::GuestFileTime;
use guest_metadata::{
    get_guest_times, get_path, put_guest_times, put_path, require_empty_path, require_empty_time_fields,
    validate_guest_share_path,
};
#[cfg(test)]
use guest_metadata::{ATIME_SECONDS_RANGE, PATH_RANGE, TIME_FLAGS_OFFSET};

/// Exact record size carried by the bounded SCM_RIGHTS channel.
pub const UPSTREAM_GRANT_FRAME_SIZE: usize = 4480;
/// A ledger response grants its storage and trusted-checkpoint channels.
pub const UPSTREAM_GRANT_MAX_FDS: usize = 2;
/// Per-worker descriptor and pending-selection ceiling with replacement overlap.
pub const UPSTREAM_GRANT_CAPACITY: usize = crate::NETWORK_TRAFFIC_GRANT_CAPACITY;
/// DNS names are at most 253 wire-text bytes without a root dot.
pub const MAX_UPSTREAM_HOST_BYTES: usize = 253;
/// Maximum relative path carried by a guest-share metadata request.
pub const MAX_GUEST_SHARE_PATH_BYTES: usize = 4096;

const MAGIC: [u8; 2] = *b"UG";
const VERSION: u8 = 10;
const MAGIC_RANGE: std::ops::Range<usize> = 0..2;
const VERSION_OFFSET: usize = 2;
const KIND_OFFSET: usize = 3;
const REQUEST_ID_RANGE: std::ops::Range<usize> = 4..12;
const RESOURCE_ID_RANGE: std::ops::Range<usize> = 12..20;
const PORT_RANGE: std::ops::Range<usize> = 20..22;
const DETAIL_RANGE: std::ops::Range<usize> = 22..24;
const NAME_LENGTH_OFFSET: usize = 24;
const NAME_RANGE: std::ops::Range<usize> = 25..25 + MAX_UPSTREAM_HOST_BYTES;
const POLICY_DIGEST_BYTES: usize = 71;
const POLICY_DIGEST_RANGE: std::ops::Range<usize> = NAME_RANGE.end..NAME_RANGE.end + POLICY_DIGEST_BYTES;
use guest_metadata::RESERVED_RANGE;

const RESOLVE_TCP: u8 = 1;
const CONNECT_TCP: u8 = 2;
const OPEN_DNS: u8 = 3;
const ADOPTED: u8 = 4;
const RELEASE: u8 = 5;
const SET_GUEST_MODE: u8 = 6;
const OPEN_LEDGER: u8 = 7;
const ATTACH_PROXY_TRAFFIC: u8 = 8;
const ATTACH_PROXY_MCP: u8 = 9;
const ATTACH_PROXY_TRACE_HINTS: u8 = 10;
const OPEN_LOOPBACK_LISTENER: u8 = 11;
const SET_GUEST_TIMES: u8 = 12;
const ATTACH_MCP_HTTP: u8 = 13;
const OPEN_MCP_AGGREGATOR: u8 = 14;
const OPEN_APPROVAL: u8 = 15;
const TCP_RESOLVED: u8 = 101;
const DESCRIPTOR_GRANTED: u8 = 102;
const DENIED: u8 = 103;
const GUEST_MODE_SET: u8 = 104;
const LEDGER_GRANTED: u8 = 105;
const PROXY_TRAFFIC_ADOPTED: u8 = 106;
const PROXY_MCP_ADOPTED: u8 = 107;
const PROXY_TRACE_HINTS_ADOPTED: u8 = 108;
const LOOPBACK_LISTENER_GRANTED: u8 = 109;
const GUEST_TIMES_SET: u8 = 110;
const MCP_HTTP_ADOPTED: u8 = 111;
const MCP_AGGREGATOR_GRANTED: u8 = 112;
const APPROVAL_GRANTED: u8 = 113;

/// Application protocol spoken over a granted TCP stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UpstreamProtocol {
    Http,
    Tls,
}

impl UpstreamProtocol {
    const fn code(self) -> u16 {
        match self {
            Self::Http => 1,
            Self::Tls => 2,
        }
    }

    fn decode(code: u16) -> Result<Self> {
        match code {
            1 => Ok(Self::Http),
            2 => Ok(Self::Tls),
            _ => bail!("invalid upstream protocol {code}"),
        }
    }
}

/// Kind of connected socket carried by a descriptor response.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UpstreamDescriptorKind {
    Tcp,
    DnsUdp,
}

/// Guest-facing service selected by a surrendered traffic descriptor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProxyTrafficService {
    Http,
    Dns,
}

impl ProxyTrafficService {
    const fn code(self) -> u16 {
        match self {
            Self::Http => 1,
            Self::Dns => 2,
        }
    }

    fn decode(code: u16) -> Result<Self> {
        match code {
            1 => Ok(Self::Http),
            2 => Ok(Self::Dns),
            _ => bail!("invalid proxy traffic service {code}"),
        }
    }
}

impl UpstreamDescriptorKind {
    const fn code(self) -> u16 {
        match self {
            Self::Tcp => 1,
            Self::DnsUdp => 2,
        }
    }

    fn decode(code: u16) -> Result<Self> {
        match code {
            1 => Ok(Self::Tcp),
            2 => Ok(Self::DnsUdp),
            _ => bail!("invalid upstream descriptor kind {code}"),
        }
    }
}

/// Bounded denial reasons. Details stay in trusted coordinator logs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UpstreamGrantDenial {
    NotConfigured,
    NotAllowed,
    Capacity,
    ResolveFailed,
    ConnectFailed,
    Revoked,
    InvalidResource,
    BindFailed,
}

impl UpstreamGrantDenial {
    const fn code(self) -> u16 {
        match self {
            Self::NotConfigured => 1,
            Self::NotAllowed => 2,
            Self::Capacity => 3,
            Self::ResolveFailed => 4,
            Self::ConnectFailed => 5,
            Self::Revoked => 6,
            Self::InvalidResource => 7,
            Self::BindFailed => 8,
        }
    }

    fn decode(code: u16) -> Result<Self> {
        match code {
            1 => Ok(Self::NotConfigured),
            2 => Ok(Self::NotAllowed),
            3 => Ok(Self::Capacity),
            4 => Ok(Self::ResolveFailed),
            5 => Ok(Self::ConnectFailed),
            6 => Ok(Self::Revoked),
            7 => Ok(Self::InvalidResource),
            8 => Ok(Self::BindFailed),
            _ => bail!("invalid upstream denial reason {code}"),
        }
    }
}

/// Worker-to-coordinator record. Request ids correlate replies; resource ids
/// are opaque coordinator-minted selections or live descriptor grants.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UpstreamGrantRequest {
    ResolveTcp {
        request_id: u64,
        protocol: UpstreamProtocol,
        host: String,
        port: u16,
    },
    ConnectTcp {
        request_id: u64,
        selection_id: u64,
    },
    OpenDns {
        request_id: u64,
        upstream_index: u16,
    },
    Adopted {
        grant_id: u64,
    },
    Release {
        resource_id: u64,
    },
    SetGuestMode {
        request_id: u64,
        relative_path: Vec<u8>,
        mode: u16,
    },
    SetGuestTimes {
        request_id: u64,
        relative_path: Vec<u8>,
        atime: GuestFileTime,
        mtime: GuestFileTime,
    },
    OpenLedger {
        request_id: u64,
    },
    /// One connected capability bound by its coordinator to this boot only.
    OpenApproval {
        request_id: u64,
    },
    AttachProxyTraffic {
        request_id: u64,
        service: ProxyTrafficService,
    },
    AttachProxyMcp {
        request_id: u64,
    },
    AttachProxyTraceHints {
        request_id: u64,
    },
    AttachMcpHttp {
        request_id: u64,
        role: McpHttpRole,
    },
    OpenMcpAggregator {
        request_id: u64,
    },
    OpenLoopbackListener {
        request_id: u64,
        port: u16,
    },
}

/// Coordinator-to-worker record. Ledger grants carry two independent fds.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UpstreamGrantResponse {
    McpAggregatorGranted {
        request_id: u64,
    },
    McpHttpAdopted {
        request_id: u64,
    },
    TcpResolved {
        request_id: u64,
        selection_id: u64,
        protocol: UpstreamProtocol,
        judged_ip: Option<IpAddr>,
        policy_digest: String,
    },
    DescriptorGranted {
        request_id: u64,
        grant_id: u64,
        kind: UpstreamDescriptorKind,
        policy_digest: String,
    },
    Denied {
        request_id: u64,
        reason: UpstreamGrantDenial,
    },
    GuestModeSet {
        request_id: u64,
    },
    GuestTimesSet {
        request_id: u64,
    },
    LedgerGranted {
        request_id: u64,
        grant: LedgerChannelGrant,
    },
    ApprovalGranted {
        request_id: u64,
    },
    ProxyTrafficAdopted {
        request_id: u64,
    },
    ProxyMcpAdopted {
        request_id: u64,
    },
    ProxyTraceHintsAdopted {
        request_id: u64,
    },
    LoopbackListenerGranted {
        request_id: u64,
        grant_id: u64,
        port: u16,
    },
}

impl UpstreamGrantRequest {
    /// Capability attachment requests carry one connected channel; ordinary
    /// resource requests never carry worker-provided descriptors.
    pub const fn expected_descriptor_count(&self) -> usize {
        match self {
            Self::AttachProxyTraffic { .. }
            | Self::AttachProxyMcp { .. }
            | Self::AttachProxyTraceHints { .. }
            | Self::AttachMcpHttp { .. } => 1,
            _ => 0,
        }
    }
}

impl UpstreamGrantResponse {
    /// Successful connected sockets carry one descriptor; ledger grants carry
    /// the direct worker channel and the trusted checkpoint channel.
    pub const fn expected_descriptor_count(&self) -> usize {
        match self {
            Self::DescriptorGranted { .. }
            | Self::LoopbackListenerGranted { .. }
            | Self::McpAggregatorGranted { .. } => 1,
            Self::ApprovalGranted { .. } => 1,
            Self::LedgerGranted { .. } => 1,
            Self::TcpResolved { .. }
            | Self::Denied { .. }
            | Self::GuestModeSet { .. }
            | Self::GuestTimesSet { .. }
            | Self::ProxyTrafficAdopted { .. }
            | Self::ProxyMcpAdopted { .. }
            | Self::McpHttpAdopted { .. }
            | Self::ProxyTraceHintsAdopted { .. } => 0,
        }
    }
}

/// Encode one coordinator response into its exact fixed-size record.
pub fn encode_upstream_grant_response(response: &UpstreamGrantResponse) -> Result<[u8; UPSTREAM_GRANT_FRAME_SIZE]> {
    let mut frame = empty_frame();
    match response {
        UpstreamGrantResponse::ApprovalGranted { request_id } => {
            require_nonzero("request id", *request_id)?;
            frame[KIND_OFFSET] = APPROVAL_GRANTED;
            put_u64(&mut frame, REQUEST_ID_RANGE, *request_id);
        }
        UpstreamGrantResponse::McpAggregatorGranted { request_id } => {
            require_nonzero("request id", *request_id)?;
            frame[KIND_OFFSET] = MCP_AGGREGATOR_GRANTED;
            put_u64(&mut frame, REQUEST_ID_RANGE, *request_id);
        }
        UpstreamGrantResponse::McpHttpAdopted { request_id } => {
            require_nonzero("request id", *request_id)?;
            frame[KIND_OFFSET] = MCP_HTTP_ADOPTED;
            put_u64(&mut frame, REQUEST_ID_RANGE, *request_id);
        }
        UpstreamGrantResponse::TcpResolved {
            request_id,
            selection_id,
            protocol,
            judged_ip,
            policy_digest,
        } => {
            require_nonzero("request id", *request_id)?;
            require_nonzero("selection id", *selection_id)?;
            frame[KIND_OFFSET] = TCP_RESOLVED;
            put_u64(&mut frame, REQUEST_ID_RANGE, *request_id);
            put_u64(&mut frame, RESOURCE_ID_RANGE, *selection_id);
            put_u16(&mut frame, DETAIL_RANGE, protocol.code());
            if let Some(address) = judged_ip {
                put_name(&mut frame, &address.to_string())?;
            }
            put_policy_digest(&mut frame, policy_digest)?;
        }
        UpstreamGrantResponse::DescriptorGranted {
            request_id,
            grant_id,
            kind,
            policy_digest,
        } => {
            require_nonzero("request id", *request_id)?;
            require_nonzero("grant id", *grant_id)?;
            frame[KIND_OFFSET] = DESCRIPTOR_GRANTED;
            put_u64(&mut frame, REQUEST_ID_RANGE, *request_id);
            put_u64(&mut frame, RESOURCE_ID_RANGE, *grant_id);
            put_u16(&mut frame, DETAIL_RANGE, kind.code());
            put_policy_digest(&mut frame, policy_digest)?;
        }
        UpstreamGrantResponse::Denied { request_id, reason } => {
            require_nonzero("request id", *request_id)?;
            frame[KIND_OFFSET] = DENIED;
            put_u64(&mut frame, REQUEST_ID_RANGE, *request_id);
            put_u16(&mut frame, DETAIL_RANGE, reason.code());
        }
        UpstreamGrantResponse::GuestModeSet { request_id } => {
            require_nonzero("request id", *request_id)?;
            frame[KIND_OFFSET] = GUEST_MODE_SET;
            put_u64(&mut frame, REQUEST_ID_RANGE, *request_id);
        }
        UpstreamGrantResponse::GuestTimesSet { request_id } => {
            require_nonzero("request id", *request_id)?;
            frame[KIND_OFFSET] = GUEST_TIMES_SET;
            put_u64(&mut frame, REQUEST_ID_RANGE, *request_id);
        }
        UpstreamGrantResponse::LedgerGranted { request_id, grant } => {
            require_nonzero("request id", *request_id)?;
            frame[KIND_OFFSET] = LEDGER_GRANTED;
            put_u64(&mut frame, REQUEST_ID_RANGE, *request_id);
            put_u64(&mut frame, RESOURCE_ID_RANGE, grant.client_id());
            put_u16(&mut frame, DETAIL_RANGE, ledger_role_code(grant.role()));
            put_name(&mut frame, &encode_ledger_generation(grant.generation()))?;
        }
        UpstreamGrantResponse::ProxyTrafficAdopted { request_id } => {
            require_nonzero("request id", *request_id)?;
            frame[KIND_OFFSET] = PROXY_TRAFFIC_ADOPTED;
            put_u64(&mut frame, REQUEST_ID_RANGE, *request_id);
        }
        UpstreamGrantResponse::ProxyMcpAdopted { request_id } => {
            require_nonzero("request id", *request_id)?;
            frame[KIND_OFFSET] = PROXY_MCP_ADOPTED;
            put_u64(&mut frame, REQUEST_ID_RANGE, *request_id);
        }
        UpstreamGrantResponse::ProxyTraceHintsAdopted { request_id } => {
            require_nonzero("request id", *request_id)?;
            frame[KIND_OFFSET] = PROXY_TRACE_HINTS_ADOPTED;
            put_u64(&mut frame, REQUEST_ID_RANGE, *request_id);
        }
        UpstreamGrantResponse::LoopbackListenerGranted {
            request_id,
            grant_id,
            port,
        } => {
            require_nonzero("request id", *request_id)?;
            require_nonzero("grant id", *grant_id)?;
            if *port == 0 {
                bail!("granted loopback listener port cannot be zero");
            }
            frame[KIND_OFFSET] = LOOPBACK_LISTENER_GRANTED;
            put_u64(&mut frame, REQUEST_ID_RANGE, *request_id);
            put_u64(&mut frame, RESOURCE_ID_RANGE, *grant_id);
            put_u16(&mut frame, PORT_RANGE, *port);
        }
    }
    Ok(frame)
}

/// Decode and fully validate one coordinator response.
pub fn decode_upstream_grant_response(frame: &[u8; UPSTREAM_GRANT_FRAME_SIZE]) -> Result<UpstreamGrantResponse> {
    validate_envelope(frame)?;
    require_empty_time_fields(frame)?;
    let request_id = get_u64(frame, REQUEST_ID_RANGE);
    let resource_id = get_u64(frame, RESOURCE_ID_RANGE);
    let port = get_u16(frame, PORT_RANGE);
    let detail = get_u16(frame, DETAIL_RANGE);
    let name = get_name(frame)?;
    let policy_digest = get_policy_digest(frame)?;
    let relative_path = get_path(frame)?;
    require_empty_path(&relative_path)?;
    require_nonzero("request id", request_id)?;
    if port != 0 && frame[KIND_OFFSET] != LOOPBACK_LISTENER_GRANTED {
        bail!("upstream response cannot carry a port");
    }
    match frame[KIND_OFFSET] {
        TCP_RESOLVED => {
            require_nonzero("selection id", resource_id)?;
            let protocol = UpstreamProtocol::decode(detail)?;
            let judged_ip = if name.is_empty() {
                None
            } else {
                let address: IpAddr = name.parse().map_err(|_| anyhow::anyhow!("invalid judged IP address"))?;
                if address.to_string() != name {
                    bail!("judged IP address is not canonical");
                }
                Some(address)
            };
            Ok(UpstreamGrantResponse::TcpResolved {
                request_id,
                selection_id: resource_id,
                protocol,
                judged_ip,
                policy_digest: require_policy_digest(policy_digest)?,
            })
        }
        DESCRIPTOR_GRANTED => {
            require_nonzero("grant id", resource_id)?;
            if !name.is_empty() {
                bail!("descriptor grant cannot carry a name");
            }
            Ok(UpstreamGrantResponse::DescriptorGranted {
                request_id,
                grant_id: resource_id,
                kind: UpstreamDescriptorKind::decode(detail)?,
                policy_digest: require_policy_digest(policy_digest)?,
            })
        }
        DENIED => {
            if resource_id != 0 || !name.is_empty() {
                bail!("denial cannot carry a resource");
            }
            if policy_digest.is_some() {
                bail!("denial cannot carry a policy digest");
            }
            Ok(UpstreamGrantResponse::Denied {
                request_id,
                reason: UpstreamGrantDenial::decode(detail)?,
            })
        }
        GUEST_MODE_SET => {
            if resource_id != 0 || detail != 0 || !name.is_empty() || policy_digest.is_some() {
                bail!("guest mode response carries unrelated fields");
            }
            Ok(UpstreamGrantResponse::GuestModeSet { request_id })
        }
        GUEST_TIMES_SET => {
            if resource_id != 0 || detail != 0 || !name.is_empty() || policy_digest.is_some() {
                bail!("guest timestamp response carries unrelated fields");
            }
            Ok(UpstreamGrantResponse::GuestTimesSet { request_id })
        }
        APPROVAL_GRANTED => {
            if resource_id != 0 || detail != 0 || !name.is_empty() || policy_digest.is_some() {
                bail!("approval grant carries unrelated fields");
            }
            Ok(UpstreamGrantResponse::ApprovalGranted { request_id })
        }
        LEDGER_GRANTED => {
            if policy_digest.is_some() {
                bail!("ledger grant cannot carry a policy digest");
            }
            let generation = decode_ledger_generation(&name)?;
            let role = decode_ledger_role(detail)?;
            let grant = LedgerChannelGrant::new(generation, resource_id, role)?;
            Ok(UpstreamGrantResponse::LedgerGranted { request_id, grant })
        }
        PROXY_TRAFFIC_ADOPTED => {
            if resource_id != 0 || detail != 0 || !name.is_empty() || policy_digest.is_some() {
                bail!("proxy traffic adoption carries unrelated fields");
            }
            Ok(UpstreamGrantResponse::ProxyTrafficAdopted { request_id })
        }
        MCP_HTTP_ADOPTED | MCP_AGGREGATOR_GRANTED => {
            if resource_id != 0 || detail != 0 || !name.is_empty() || policy_digest.is_some() {
                bail!("MCP capability adoption carries unrelated fields");
            }
            Ok(if frame[KIND_OFFSET] == MCP_HTTP_ADOPTED {
                UpstreamGrantResponse::McpHttpAdopted { request_id }
            } else {
                UpstreamGrantResponse::McpAggregatorGranted { request_id }
            })
        }
        PROXY_MCP_ADOPTED => {
            if resource_id != 0 || detail != 0 || !name.is_empty() || policy_digest.is_some() {
                bail!("proxy MCP adoption carries unrelated fields");
            }
            Ok(UpstreamGrantResponse::ProxyMcpAdopted { request_id })
        }
        PROXY_TRACE_HINTS_ADOPTED => {
            if resource_id != 0 || detail != 0 || !name.is_empty() || policy_digest.is_some() {
                bail!("proxy trace-hint adoption carries unrelated fields");
            }
            Ok(UpstreamGrantResponse::ProxyTraceHintsAdopted { request_id })
        }
        LOOPBACK_LISTENER_GRANTED => {
            require_nonzero("grant id", resource_id)?;
            if port == 0 || detail != 0 || !name.is_empty() || policy_digest.is_some() {
                bail!("loopback listener grant carries invalid fields");
            }
            Ok(UpstreamGrantResponse::LoopbackListenerGranted {
                request_id,
                grant_id: resource_id,
                port,
            })
        }
        kind => bail!("invalid upstream response kind {kind}"),
    }
}

fn empty_frame() -> [u8; UPSTREAM_GRANT_FRAME_SIZE] {
    let mut frame = [0; UPSTREAM_GRANT_FRAME_SIZE];
    frame[MAGIC_RANGE].copy_from_slice(&MAGIC);
    frame[VERSION_OFFSET] = VERSION;
    frame
}

fn validate_envelope(frame: &[u8; UPSTREAM_GRANT_FRAME_SIZE]) -> Result<()> {
    if frame[MAGIC_RANGE] != MAGIC {
        bail!("invalid upstream grant magic");
    }
    if frame[VERSION_OFFSET] != VERSION {
        bail!("unsupported upstream grant version {}", frame[VERSION_OFFSET]);
    }
    if frame[RESERVED_RANGE].iter().any(|byte| *byte != 0) {
        bail!("upstream grant reserved bytes are nonzero");
    }
    Ok(())
}

fn validate_normalized_host(host: &str) -> Result<()> {
    let bytes = host.as_bytes();
    if bytes.is_empty() || bytes.len() > MAX_UPSTREAM_HOST_BYTES {
        bail!("invalid upstream host length {}", bytes.len());
    }
    if !host.is_ascii() || host != host.to_ascii_lowercase() || host.ends_with('.') {
        bail!("upstream host is not normalized");
    }
    if host.contains(':') {
        let address: IpAddr = host
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid upstream IP literal"))?;
        if address.to_string() != host {
            bail!("upstream IP literal is not canonical");
        }
        return Ok(());
    }
    for label in host.split('.') {
        if label.is_empty()
            || label.len() > 63
            || !label
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_'))
        {
            bail!("invalid normalized upstream host");
        }
    }
    Ok(())
}

fn put_name(frame: &mut [u8; UPSTREAM_GRANT_FRAME_SIZE], name: &str) -> Result<()> {
    let length = u8::try_from(name.len()).map_err(|_| anyhow::anyhow!("upstream name is too long"))?;
    if usize::from(length) > MAX_UPSTREAM_HOST_BYTES {
        bail!("upstream name is too long");
    }
    frame[NAME_LENGTH_OFFSET] = length;
    frame[NAME_RANGE.start..NAME_RANGE.start + usize::from(length)].copy_from_slice(name.as_bytes());
    Ok(())
}

fn get_name(frame: &[u8; UPSTREAM_GRANT_FRAME_SIZE]) -> Result<String> {
    let length = usize::from(frame[NAME_LENGTH_OFFSET]);
    if length > NAME_RANGE.len() {
        bail!("upstream name length exceeds its field");
    }
    let (used, padding) = frame[NAME_RANGE].split_at(length);
    if padding.iter().any(|byte| *byte != 0) {
        bail!("upstream name padding is nonzero");
    }
    String::from_utf8(used.to_vec()).map_err(|_| anyhow::anyhow!("upstream name is not UTF-8"))
}

fn put_policy_digest(frame: &mut [u8; UPSTREAM_GRANT_FRAME_SIZE], digest: &str) -> Result<()> {
    validate_policy_digest(digest)?;
    frame[POLICY_DIGEST_RANGE].copy_from_slice(digest.as_bytes());
    Ok(())
}

fn get_policy_digest(frame: &[u8; UPSTREAM_GRANT_FRAME_SIZE]) -> Result<Option<String>> {
    let bytes = &frame[POLICY_DIGEST_RANGE];
    if bytes.iter().all(|byte| *byte == 0) {
        return Ok(None);
    }
    let digest = std::str::from_utf8(bytes).map_err(|_| anyhow::anyhow!("policy digest is not UTF-8"))?;
    validate_policy_digest(digest)?;
    Ok(Some(digest.to_owned()))
}

fn require_policy_digest(digest: Option<String>) -> Result<String> {
    digest.ok_or_else(|| anyhow::anyhow!("successful upstream response lacks a policy digest"))
}

fn require_empty_policy_digest(frame: &[u8; UPSTREAM_GRANT_FRAME_SIZE]) -> Result<()> {
    if frame[POLICY_DIGEST_RANGE].iter().any(|byte| *byte != 0) {
        bail!("upstream request cannot carry a policy digest");
    }
    Ok(())
}

fn ledger_role_code(role: LedgerClientRole) -> u16 {
    match role {
        LedgerClientRole::VmOwner => 1,
        LedgerClientRole::Proxy => 2,
        LedgerClientRole::Coordinator => 3,
        LedgerClientRole::Reader => 4,
        LedgerClientRole::Maintainer => 5,
        LedgerClientRole::Supervisor => 6,
    }
}

fn decode_ledger_role(code: u16) -> Result<LedgerClientRole> {
    match code {
        1 => Ok(LedgerClientRole::VmOwner),
        2 => Ok(LedgerClientRole::Proxy),
        3 => Ok(LedgerClientRole::Coordinator),
        4 => Ok(LedgerClientRole::Reader),
        5 => Ok(LedgerClientRole::Maintainer),
        6 => Ok(LedgerClientRole::Supervisor),
        _ => bail!("invalid ledger client role {code}"),
    }
}

fn encode_ledger_generation(generation: LedgerGeneration) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(32);
    for byte in generation.as_bytes() {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

fn decode_ledger_generation(encoded: &str) -> Result<LedgerGeneration> {
    if encoded.len() != 32
        || !encoded
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        bail!("ledger generation must contain 32 lowercase hexadecimal bytes");
    }
    let mut bytes = [0_u8; 16];
    for (index, pair) in encoded.as_bytes().chunks_exact(2).enumerate() {
        bytes[index] = (decode_hex_nibble(pair[0])? << 4) | decode_hex_nibble(pair[1])?;
    }
    Ok(LedgerGeneration::new(bytes))
}

fn decode_hex_nibble(byte: u8) -> Result<u8> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => bail!("invalid lowercase hexadecimal byte"),
    }
}

fn validate_policy_digest(digest: &str) -> Result<()> {
    let Some(hash) = digest.strip_prefix("blake3:") else {
        bail!("policy digest must have the blake3 scheme");
    };
    if hash.len() != 64
        || !hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        bail!("policy digest must contain 64 lowercase hexadecimal bytes");
    }
    Ok(())
}

fn require_nonzero(label: &str, value: u64) -> Result<()> {
    if value == 0 {
        bail!("{label} cannot be zero");
    }
    Ok(())
}

fn require_empty_fields(port: u16, detail: u16, name: &str) -> Result<()> {
    if port != 0 || detail != 0 || !name.is_empty() {
        bail!("ownership record carries unrelated fields");
    }
    Ok(())
}

fn put_u16(frame: &mut [u8; UPSTREAM_GRANT_FRAME_SIZE], range: std::ops::Range<usize>, value: u16) {
    frame[range].copy_from_slice(&value.to_be_bytes());
}

fn get_u16(frame: &[u8; UPSTREAM_GRANT_FRAME_SIZE], range: std::ops::Range<usize>) -> u16 {
    u16::from_be_bytes(frame[range].try_into().unwrap())
}

fn put_u32(frame: &mut [u8; UPSTREAM_GRANT_FRAME_SIZE], range: std::ops::Range<usize>, value: u32) {
    frame[range].copy_from_slice(&value.to_be_bytes());
}

fn get_u32(frame: &[u8; UPSTREAM_GRANT_FRAME_SIZE], range: std::ops::Range<usize>) -> u32 {
    u32::from_be_bytes(frame[range].try_into().unwrap())
}

fn put_i64(frame: &mut [u8; UPSTREAM_GRANT_FRAME_SIZE], range: std::ops::Range<usize>, value: i64) {
    frame[range].copy_from_slice(&value.to_be_bytes());
}

fn get_i64(frame: &[u8; UPSTREAM_GRANT_FRAME_SIZE], range: std::ops::Range<usize>) -> i64 {
    i64::from_be_bytes(frame[range].try_into().unwrap())
}

fn put_u64(frame: &mut [u8; UPSTREAM_GRANT_FRAME_SIZE], range: std::ops::Range<usize>, value: u64) {
    frame[range].copy_from_slice(&value.to_be_bytes());
}

fn get_u64(frame: &[u8; UPSTREAM_GRANT_FRAME_SIZE], range: std::ops::Range<usize>) -> u64 {
    u64::from_be_bytes(frame[range].try_into().unwrap())
}

#[cfg(test)]
mod tests;
