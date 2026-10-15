//! Worker-to-coordinator request codec.

use super::*;

/// Encode one worker request into its exact fixed-size record.
pub fn encode_upstream_grant_request(request: &UpstreamGrantRequest) -> Result<[u8; UPSTREAM_GRANT_FRAME_SIZE]> {
    let mut frame = empty_frame();
    match request {
        UpstreamGrantRequest::OpenMcpAggregator { request_id } => {
            require_nonzero("request id", *request_id)?;
            frame[KIND_OFFSET] = OPEN_MCP_AGGREGATOR;
            put_u64(&mut frame, REQUEST_ID_RANGE, *request_id);
        }
        UpstreamGrantRequest::AttachMcpHttp { request_id, role } => {
            require_nonzero("request id", *request_id)?;
            frame[KIND_OFFSET] = ATTACH_MCP_HTTP;
            put_u64(&mut frame, REQUEST_ID_RANGE, *request_id);
            put_u16(&mut frame, DETAIL_RANGE, *role as u16);
        }
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
        UpstreamGrantRequest::SetGuestTimes {
            request_id,
            relative_path,
            atime,
            mtime,
        } => {
            require_nonzero("request id", *request_id)?;
            validate_guest_share_path(relative_path)?;
            if matches!((atime, mtime), (GuestFileTime::Omit, GuestFileTime::Omit)) {
                bail!("guest timestamp request changes no timestamps");
            }
            frame[KIND_OFFSET] = SET_GUEST_TIMES;
            put_u64(&mut frame, REQUEST_ID_RANGE, *request_id);
            put_path(&mut frame, relative_path)?;
            put_guest_times(&mut frame, *atime, *mtime)?;
        }
        UpstreamGrantRequest::OpenLedger { request_id } => {
            require_nonzero("request id", *request_id)?;
            frame[KIND_OFFSET] = OPEN_LEDGER;
            put_u64(&mut frame, REQUEST_ID_RANGE, *request_id);
        }
        UpstreamGrantRequest::AttachProxyTraffic { request_id, service } => {
            require_nonzero("request id", *request_id)?;
            frame[KIND_OFFSET] = ATTACH_PROXY_TRAFFIC;
            put_u64(&mut frame, REQUEST_ID_RANGE, *request_id);
            put_u16(&mut frame, DETAIL_RANGE, service.code());
        }
        UpstreamGrantRequest::AttachProxyMcp { request_id } => {
            require_nonzero("request id", *request_id)?;
            frame[KIND_OFFSET] = ATTACH_PROXY_MCP;
            put_u64(&mut frame, REQUEST_ID_RANGE, *request_id);
        }
        UpstreamGrantRequest::AttachProxyTraceHints { request_id } => {
            require_nonzero("request id", *request_id)?;
            frame[KIND_OFFSET] = ATTACH_PROXY_TRACE_HINTS;
            put_u64(&mut frame, REQUEST_ID_RANGE, *request_id);
        }
        UpstreamGrantRequest::OpenLoopbackListener { request_id, port } => {
            require_nonzero("request id", *request_id)?;
            frame[KIND_OFFSET] = OPEN_LOOPBACK_LISTENER;
            put_u64(&mut frame, REQUEST_ID_RANGE, *request_id);
            put_u16(&mut frame, PORT_RANGE, *port);
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
    if frame[KIND_OFFSET] != SET_GUEST_TIMES {
        require_empty_time_fields(frame)?;
    }
    match frame[KIND_OFFSET] {
        OPEN_MCP_AGGREGATOR => {
            require_nonzero("request id", request_id)?;
            if resource_id != 0 {
                bail!("MCP spawn handoff carries a resource id");
            }
            require_empty_fields(port, detail, &name)?;
            require_empty_path(&relative_path)?;
            Ok(UpstreamGrantRequest::OpenMcpAggregator { request_id })
        }
        ATTACH_MCP_HTTP => {
            require_nonzero("request id", request_id)?;
            if resource_id != 0 || port != 0 || !name.is_empty() {
                bail!("MCP HTTP handoff carries unrelated fields");
            }
            require_empty_path(&relative_path)?;
            Ok(UpstreamGrantRequest::AttachMcpHttp {
                request_id,
                role: McpHttpRole::decode(detail)?,
            })
        }
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
        SET_GUEST_TIMES => {
            require_nonzero("request id", request_id)?;
            if resource_id != 0 || port != 0 || detail != 0 || !name.is_empty() {
                bail!("invalid guest timestamp request fields");
            }
            validate_guest_share_path(&relative_path)?;
            let (atime, mtime) = get_guest_times(frame)?;
            if matches!((atime, mtime), (GuestFileTime::Omit, GuestFileTime::Omit)) {
                bail!("guest timestamp request changes no timestamps");
            }
            Ok(UpstreamGrantRequest::SetGuestTimes {
                request_id,
                relative_path,
                atime,
                mtime,
            })
        }
        OPEN_LEDGER => {
            require_nonzero("request id", request_id)?;
            if resource_id != 0 {
                bail!("ledger request cannot carry a resource");
            }
            require_empty_fields(port, detail, &name)?;
            require_empty_path(&relative_path)?;
            Ok(UpstreamGrantRequest::OpenLedger { request_id })
        }
        ATTACH_PROXY_TRAFFIC => {
            require_nonzero("request id", request_id)?;
            if resource_id != 0 || port != 0 || !name.is_empty() {
                bail!("proxy traffic request carries unrelated fields");
            }
            require_empty_path(&relative_path)?;
            Ok(UpstreamGrantRequest::AttachProxyTraffic {
                request_id,
                service: ProxyTrafficService::decode(detail)?,
            })
        }
        ATTACH_PROXY_MCP => {
            require_nonzero("request id", request_id)?;
            if resource_id != 0 {
                bail!("proxy MCP request carries a resource");
            }
            require_empty_fields(port, detail, &name)?;
            require_empty_path(&relative_path)?;
            Ok(UpstreamGrantRequest::AttachProxyMcp { request_id })
        }
        ATTACH_PROXY_TRACE_HINTS => {
            require_nonzero("request id", request_id)?;
            if resource_id != 0 {
                bail!("proxy trace-hint request carries a resource");
            }
            require_empty_fields(port, detail, &name)?;
            require_empty_path(&relative_path)?;
            Ok(UpstreamGrantRequest::AttachProxyTraceHints { request_id })
        }
        OPEN_LOOPBACK_LISTENER => {
            require_nonzero("request id", request_id)?;
            if resource_id != 0 || detail != 0 || !name.is_empty() {
                bail!("loopback listener request carries unrelated fields");
            }
            require_empty_path(&relative_path)?;
            Ok(UpstreamGrantRequest::OpenLoopbackListener { request_id, port })
        }
        kind => bail!("invalid upstream request kind {kind}"),
    }
}
