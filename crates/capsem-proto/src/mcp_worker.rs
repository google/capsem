//! Bootstrap and bounded spawn records for one native MCP worker generation.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

pub const MCP_SERVER_HEADER: &str = "x-capsem-mcp-server";
pub const MCP_ADMISSION_HEADER: &str = "x-capsem-mcp-admission";

/// Produced only by the generation-bound HTTP broker, after evaluating its
/// current policy and the pinned upstream addresses. A worker never trusts an
/// upstream header with this name: the broker replaces it unconditionally.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum McpHttpAdmission {
    Allowed {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rule_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    Denied {
        reason: String,
    },
}

pub const MCP_WORKER_FRAME_SIZE: usize = 24;
pub const MCP_WORKER_MAX_FDS: usize = 2;
pub const MCP_SPAWN_FRAME_SIZE: usize = 16;
pub const MCP_SPAWN_MAX_FDS: usize = 1;

/// An MCP HTTP capability is either configured-server transport or builtin
/// tool egress. Neither role carries ledger, lifecycle or credential-store APIs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum McpHttpRole {
    Aggregator = 1,
    Builtin = 2,
}

impl McpHttpRole {
    pub(crate) fn decode(code: u16) -> Result<Self> {
        match code {
            1 => Ok(Self::Aggregator),
            2 => Ok(Self::Builtin),
            _ => bail!("unknown MCP HTTP role {code}"),
        }
    }
}

/// The trusted parent supplies the actual registered VM spawn generation.
/// The connected descriptor establishes authority; this identity fences
/// readiness and accidental cross-generation handoff, never replaces the FD.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct McpWorkerBootstrap {
    pub role: McpHttpRole,
    pub generation: [u8; 16],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpWorkerRecord {
    Bootstrap(McpWorkerBootstrap),
    Ready(McpWorkerBootstrap),
}

impl McpWorkerRecord {
    pub fn expected_descriptor_count(self) -> usize {
        match self {
            Self::Bootstrap(McpWorkerBootstrap {
                role: McpHttpRole::Aggregator,
                ..
            }) => 2,
            Self::Bootstrap(McpWorkerBootstrap {
                role: McpHttpRole::Builtin,
                ..
            }) => 1,
            Self::Ready(_) => 0,
        }
    }
}

pub fn encode_worker_record(record: McpWorkerRecord) -> Result<[u8; MCP_WORKER_FRAME_SIZE]> {
    let (kind, bootstrap) = match record {
        McpWorkerRecord::Bootstrap(bootstrap) => (1, bootstrap),
        McpWorkerRecord::Ready(bootstrap) => (2, bootstrap),
    };
    if bootstrap.generation == [0; 16] {
        bail!("MCP worker generation is zero");
    }
    let mut frame = [0; MCP_WORKER_FRAME_SIZE];
    frame[..2].copy_from_slice(b"MW");
    frame[2] = 1;
    frame[3] = kind;
    frame[4] = bootstrap.role as u8;
    frame[8..].copy_from_slice(&bootstrap.generation);
    Ok(frame)
}

pub fn decode_worker_record(frame: &[u8; MCP_WORKER_FRAME_SIZE]) -> Result<McpWorkerRecord> {
    if &frame[..2] != b"MW" || frame[2] != 1 || frame[5..8] != [0; 3] {
        bail!("invalid MCP worker envelope");
    }
    let bootstrap = McpWorkerBootstrap {
        role: McpHttpRole::decode(u16::from(frame[4]))?,
        generation: frame[8..].try_into().expect("fixed generation size"),
    };
    if bootstrap.generation == [0; 16] {
        bail!("MCP worker generation is zero");
    }
    match frame[3] {
        1 => Ok(McpWorkerRecord::Bootstrap(bootstrap)),
        2 => Ok(McpWorkerRecord::Ready(bootstrap)),
        kind => bail!("unknown MCP worker record {kind}"),
    }
}

/// The worker can request a peer of the parent's own builtin definition.
/// No worker-supplied command, environment, path or arbitrary argv crosses this API.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpSpawnRecord {
    Builtin { request_id: u64, peer_index: u32 },
    Granted { request_id: u64 },
    Denied { request_id: u64 },
}

impl McpSpawnRecord {
    pub fn expected_descriptor_count(self) -> usize {
        usize::from(matches!(self, Self::Granted { .. }))
    }
}

pub fn encode_spawn_record(record: McpSpawnRecord) -> Result<[u8; MCP_SPAWN_FRAME_SIZE]> {
    let (kind, request_id, peer_index) = match record {
        McpSpawnRecord::Builtin { request_id, peer_index } => (1, request_id, peer_index),
        McpSpawnRecord::Granted { request_id } => (2, request_id, 0),
        McpSpawnRecord::Denied { request_id } => (3, request_id, 0),
    };
    if request_id == 0 {
        bail!("MCP spawn request id is zero");
    }
    let mut frame = [0; MCP_SPAWN_FRAME_SIZE];
    frame[..2].copy_from_slice(b"MS");
    frame[2] = 1;
    frame[3] = kind;
    frame[4..12].copy_from_slice(&request_id.to_be_bytes());
    frame[12..].copy_from_slice(&peer_index.to_be_bytes());
    Ok(frame)
}

pub fn decode_spawn_record(frame: &[u8; MCP_SPAWN_FRAME_SIZE]) -> Result<McpSpawnRecord> {
    if &frame[..2] != b"MS" || frame[2] != 1 {
        bail!("invalid MCP spawn envelope");
    }
    let request_id = u64::from_be_bytes(frame[4..12].try_into().expect("fixed request size"));
    if request_id == 0 {
        bail!("MCP spawn request id is zero");
    }
    let peer_index = u32::from_be_bytes(frame[12..].try_into().expect("fixed peer size"));
    match frame[3] {
        1 => Ok(McpSpawnRecord::Builtin { request_id, peer_index }),
        2 if peer_index == 0 => Ok(McpSpawnRecord::Granted { request_id }),
        3 if peer_index == 0 => Ok(McpSpawnRecord::Denied { request_id }),
        _ => bail!("invalid MCP spawn kind or reserved fields"),
    }
}

#[cfg(test)]
mod tests;
