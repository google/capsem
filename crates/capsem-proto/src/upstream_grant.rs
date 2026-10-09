//! Fixed, versioned records for coordinator-minted upstream descriptors.
//!
//! A worker first asks the trusted coordinator to resolve a TCP target. The
//! response carries an opaque selection id, the address policy must judge and
//! the effective HTTP/TLS protocol. Only after policy admits that selection
//! does the worker ask the coordinator to connect it. DNS requests name a
//! configured upstream by index, never by worker-supplied address.

use std::net::IpAddr;

use anyhow::{bail, Result};

/// Exact record size carried by the bounded SCM_RIGHTS channel.
pub const UPSTREAM_GRANT_FRAME_SIZE: usize = 4480;
/// A response grants at most one connected TCP or UDP descriptor.
pub const UPSTREAM_GRANT_MAX_FDS: usize = 1;
/// DNS names are at most 253 wire-text bytes without a root dot.
pub const MAX_UPSTREAM_HOST_BYTES: usize = 253;
/// Maximum relative path carried by a guest-share metadata request.
pub const MAX_GUEST_SHARE_PATH_BYTES: usize = 4096;

const MAGIC: [u8; 2] = *b"UG";
const VERSION: u8 = 1;
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
const PATH_LENGTH_RANGE: std::ops::Range<usize> = POLICY_DIGEST_RANGE.end..POLICY_DIGEST_RANGE.end + 2;
const PATH_RANGE: std::ops::Range<usize> = PATH_LENGTH_RANGE.end..PATH_LENGTH_RANGE.end + MAX_GUEST_SHARE_PATH_BYTES;
const RESERVED_RANGE: std::ops::Range<usize> = PATH_RANGE.end..UPSTREAM_GRANT_FRAME_SIZE;

const RESOLVE_TCP: u8 = 1;
const CONNECT_TCP: u8 = 2;
const OPEN_DNS: u8 = 3;
const ADOPTED: u8 = 4;
const RELEASE: u8 = 5;
const SET_GUEST_MODE: u8 = 6;
const TCP_RESOLVED: u8 = 101;
const DESCRIPTOR_GRANTED: u8 = 102;
const DENIED: u8 = 103;
const GUEST_MODE_SET: u8 = 104;

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
}

/// Coordinator-to-worker record. Only `DescriptorGranted` carries one fd.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UpstreamGrantResponse {
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
}

impl UpstreamGrantRequest {
    /// Requests never carry descriptors from the untrusted worker.
    pub const fn expected_descriptor_count(&self) -> usize {
        0
    }
}

impl UpstreamGrantResponse {
    /// Only a successful connected-socket grant carries a descriptor.
    pub const fn expected_descriptor_count(&self) -> usize {
        match self {
            Self::DescriptorGranted { .. } => 1,
            Self::TcpResolved { .. } | Self::Denied { .. } | Self::GuestModeSet { .. } => 0,
        }
    }
}

/// Encode one worker request into its exact fixed-size record.
pub fn encode_upstream_grant_request(request: &UpstreamGrantRequest) -> Result<[u8; UPSTREAM_GRANT_FRAME_SIZE]> {
    let mut frame = empty_frame();
    match request {
        UpstreamGrantRequest::ResolveTcp {
            request_id,
            protocol,
            host,
            port,
        } => {
            require_nonzero("request id", *request_id)?;
            if *port == 0 {
                bail!("upstream port cannot be zero");
            }
            validate_normalized_host(host)?;
            frame[KIND_OFFSET] = RESOLVE_TCP;
            put_u64(&mut frame, REQUEST_ID_RANGE, *request_id);
            put_u16(&mut frame, PORT_RANGE, *port);
            put_u16(&mut frame, DETAIL_RANGE, protocol.code());
            put_name(&mut frame, host)?;
        }
        UpstreamGrantRequest::ConnectTcp {
            request_id,
            selection_id,
        } => {
            require_nonzero("request id", *request_id)?;
            require_nonzero("selection id", *selection_id)?;
            frame[KIND_OFFSET] = CONNECT_TCP;
            put_u64(&mut frame, REQUEST_ID_RANGE, *request_id);
            put_u64(&mut frame, RESOURCE_ID_RANGE, *selection_id);
        }
        UpstreamGrantRequest::OpenDns {
            request_id,
            upstream_index,
        } => {
            require_nonzero("request id", *request_id)?;
            frame[KIND_OFFSET] = OPEN_DNS;
            put_u64(&mut frame, REQUEST_ID_RANGE, *request_id);
            put_u16(&mut frame, DETAIL_RANGE, *upstream_index);
        }
        UpstreamGrantRequest::Adopted { grant_id } => {
            require_nonzero("grant id", *grant_id)?;
            frame[KIND_OFFSET] = ADOPTED;
            put_u64(&mut frame, RESOURCE_ID_RANGE, *grant_id);
        }
        UpstreamGrantRequest::Release { resource_id } => {
            require_nonzero("resource id", *resource_id)?;
            frame[KIND_OFFSET] = RELEASE;
            put_u64(&mut frame, RESOURCE_ID_RANGE, *resource_id);
        }
        UpstreamGrantRequest::SetGuestMode {
            request_id,
            relative_path,
            mode,
        } => {
            require_nonzero("request id", *request_id)?;
            if *mode & !0o7777 != 0 {
                bail!("guest mode contains non-permission bits");
            }
            validate_guest_share_path(relative_path)?;
            frame[KIND_OFFSET] = SET_GUEST_MODE;
            put_u64(&mut frame, REQUEST_ID_RANGE, *request_id);
            put_u16(&mut frame, DETAIL_RANGE, *mode);
            put_path(&mut frame, relative_path)?;
        }
    }
    Ok(frame)
}

/// Decode and fully validate one worker request.
pub fn decode_upstream_grant_request(frame: &[u8; UPSTREAM_GRANT_FRAME_SIZE]) -> Result<UpstreamGrantRequest> {
    validate_envelope(frame)?;
    require_empty_policy_digest(frame)?;
    let request_id = get_u64(frame, REQUEST_ID_RANGE);
    let resource_id = get_u64(frame, RESOURCE_ID_RANGE);
    let port = get_u16(frame, PORT_RANGE);
    let detail = get_u16(frame, DETAIL_RANGE);
    let name = get_name(frame)?;
    let relative_path = get_path(frame)?;
    match frame[KIND_OFFSET] {
        RESOLVE_TCP => {
            require_nonzero("request id", request_id)?;
            if resource_id != 0 || port == 0 {
                bail!("invalid TCP resolution fields");
            }
            let protocol = UpstreamProtocol::decode(detail)?;
            validate_normalized_host(&name)?;
            require_empty_path(&relative_path)?;
            Ok(UpstreamGrantRequest::ResolveTcp {
                request_id,
                protocol,
                host: name,
                port,
            })
        }
        CONNECT_TCP => {
            require_nonzero("request id", request_id)?;
            require_nonzero("selection id", resource_id)?;
            require_empty_fields(port, detail, &name)?;
            require_empty_path(&relative_path)?;
            Ok(UpstreamGrantRequest::ConnectTcp {
                request_id,
                selection_id: resource_id,
            })
        }
        OPEN_DNS => {
            require_nonzero("request id", request_id)?;
            if resource_id != 0 || port != 0 || !name.is_empty() {
                bail!("invalid DNS grant fields");
            }
            require_empty_path(&relative_path)?;
            Ok(UpstreamGrantRequest::OpenDns {
                request_id,
                upstream_index: detail,
            })
        }
        ADOPTED => {
            require_nonzero("grant id", resource_id)?;
            if request_id != 0 {
                bail!("adoption cannot carry a request id");
            }
            require_empty_fields(port, detail, &name)?;
            require_empty_path(&relative_path)?;
            Ok(UpstreamGrantRequest::Adopted { grant_id: resource_id })
        }
        RELEASE => {
            require_nonzero("resource id", resource_id)?;
            if request_id != 0 {
                bail!("release cannot carry a request id");
            }
            require_empty_fields(port, detail, &name)?;
            require_empty_path(&relative_path)?;
            Ok(UpstreamGrantRequest::Release { resource_id })
        }
        SET_GUEST_MODE => {
            require_nonzero("request id", request_id)?;
            if resource_id != 0 || port != 0 || !name.is_empty() || detail & !0o7777 != 0 {
                bail!("invalid guest mode request fields");
            }
            validate_guest_share_path(&relative_path)?;
            Ok(UpstreamGrantRequest::SetGuestMode {
                request_id,
                relative_path,
                mode: detail,
            })
        }
        kind => bail!("invalid upstream request kind {kind}"),
    }
}

/// Encode one coordinator response into its exact fixed-size record.
pub fn encode_upstream_grant_response(response: &UpstreamGrantResponse) -> Result<[u8; UPSTREAM_GRANT_FRAME_SIZE]> {
    let mut frame = empty_frame();
    match response {
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
    }
    Ok(frame)
}

/// Decode and fully validate one coordinator response.
pub fn decode_upstream_grant_response(frame: &[u8; UPSTREAM_GRANT_FRAME_SIZE]) -> Result<UpstreamGrantResponse> {
    validate_envelope(frame)?;
    let request_id = get_u64(frame, REQUEST_ID_RANGE);
    let resource_id = get_u64(frame, RESOURCE_ID_RANGE);
    let port = get_u16(frame, PORT_RANGE);
    let detail = get_u16(frame, DETAIL_RANGE);
    let name = get_name(frame)?;
    let policy_digest = get_policy_digest(frame)?;
    let relative_path = get_path(frame)?;
    require_empty_path(&relative_path)?;
    require_nonzero("request id", request_id)?;
    if port != 0 {
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

fn validate_guest_share_path(path: &[u8]) -> Result<()> {
    if path.len() > MAX_GUEST_SHARE_PATH_BYTES || path.contains(&0) || path.first() == Some(&b'/') {
        bail!("invalid guest-share path");
    }
    if !path.is_empty()
        && path
            .split(|byte| *byte == b'/')
            .any(|component| component.is_empty() || component == b"." || component == b"..")
    {
        bail!("guest-share path is not normalized");
    }
    Ok(())
}

fn put_path(frame: &mut [u8; UPSTREAM_GRANT_FRAME_SIZE], path: &[u8]) -> Result<()> {
    validate_guest_share_path(path)?;
    let length = u16::try_from(path.len()).map_err(|_| anyhow::anyhow!("guest-share path is too long"))?;
    put_u16(frame, PATH_LENGTH_RANGE, length);
    frame[PATH_RANGE.start..PATH_RANGE.start + path.len()].copy_from_slice(path);
    Ok(())
}

fn get_path(frame: &[u8; UPSTREAM_GRANT_FRAME_SIZE]) -> Result<Vec<u8>> {
    let length = usize::from(get_u16(frame, PATH_LENGTH_RANGE));
    if length > PATH_RANGE.len() {
        bail!("guest-share path length exceeds its field");
    }
    let (used, padding) = frame[PATH_RANGE].split_at(length);
    if padding.iter().any(|byte| *byte != 0) {
        bail!("guest-share path padding is nonzero");
    }
    Ok(used.to_vec())
}

fn require_empty_path(path: &[u8]) -> Result<()> {
    if !path.is_empty() {
        bail!("record carries an unrelated guest-share path");
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

fn validate_policy_digest(digest: &str) -> Result<()> {
    let Some(hash) = digest.strip_prefix("blake3:") else {
        bail!("policy digest must use blake3");
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

fn put_u64(frame: &mut [u8; UPSTREAM_GRANT_FRAME_SIZE], range: std::ops::Range<usize>, value: u64) {
    frame[range].copy_from_slice(&value.to_be_bytes());
}

fn get_u64(frame: &[u8; UPSTREAM_GRANT_FRAME_SIZE], range: std::ops::Range<usize>) -> u64 {
    u64::from_be_bytes(frame[range].try_into().unwrap())
}

#[cfg(test)]
mod tests;
