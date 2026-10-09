//! Transport-independent operations and replies for one session ledger.

use std::collections::BTreeMap;

use capsem_foundation::ipc_channel::MAX_IPC_FRAME_SIZE;
use capsem_proto::ledger::{
    LedgerCapability, LedgerHello, LedgerOperation, LedgerProtocolError, LedgerRequest, LedgerWelcome,
};
use capsem_proto::ledger_counters::LedgerCounters;
use serde::{Deserialize, Serialize};

use crate::{BodyDirection, RetainOutcome, WriteOp};

/// Leave room inside the 16 MiB frame for request map keys and ids.
pub const MAX_LEDGER_OPERATION_BYTES: usize = MAX_IPC_FRAME_SIZE as usize - 4 * 1024;
/// Every large result streams in independently bounded pieces.
pub const MAX_LEDGER_STREAM_CHUNK_BYTES: usize = 256 * 1024;
pub const MAX_LEDGER_QUERY_ROWS: usize = 2_000;
const MAX_QUERY_TEXT_BYTES: usize = 4 * 1024;
const MAX_LEDGER_COLUMNS: usize = 128;
const MAX_LEDGER_COLUMN_BYTES: usize = 128;
const MAX_LEDGER_CELL_BYTES: usize = 1024 * 1024;
const MAX_LEDGER_RESULT_SETS: usize = 16;
const MAX_EVENT_ID_BYTES: usize = 12;
const MAX_TIMESTAMP_BYTES: usize = 64;
const SNAPSHOT_ID_BYTES: usize = 16;

pub type LedgerResponse = capsem_proto::ledger::LedgerResponse<LedgerReply>;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LedgerClientMessage {
    Hello { hello: LedgerHello },
    Request { request: LedgerRequest<LedgerCommand> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LedgerServerMessage {
    Welcome { welcome: LedgerWelcome },
    Response { response: LedgerResponse },
}

/// Every operation accepted by the ledger worker. None selects a session or
/// filesystem path; the authenticated descriptor already selects both.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LedgerCommand {
    Admit {
        event: Box<WriteOp>,
    },
    /// Make every accepted event through the returned admission id durable.
    Flush,
    Query {
        query: LedgerQuery,
    },
    ReadBodies {
        event_id: String,
    },
    Counters,
    Retain {
        cutoff: String,
    },
    /// Stream a WARC as bounded `WarcChunk` replies followed by `WarcComplete`.
    ExportWarc,
    /// Create a coherent ledger copy in a worker-owned staging directory.
    Snapshot {
        snapshot_id: [u8; SNAPSHOT_ID_BYTES],
    },
    Shutdown,
}

impl LedgerOperation for LedgerCommand {
    fn capability(&self) -> LedgerCapability {
        match self {
            Self::Admit { .. } => LedgerCapability::Admit,
            Self::Flush => LedgerCapability::Flush,
            Self::Query { .. } | Self::ReadBodies { .. } | Self::Counters => LedgerCapability::Read,
            Self::Retain { .. } => LedgerCapability::Retain,
            Self::ExportWarc => LedgerCapability::Export,
            Self::Snapshot { .. } => LedgerCapability::Snapshot,
            Self::Shutdown => LedgerCapability::Shutdown,
        }
    }

    fn validate(&self) -> Result<(), LedgerProtocolError> {
        match self {
            Self::Query { query } => query.validate()?,
            Self::ReadBodies { event_id } if !valid_event_id(event_id) => {
                return Err(LedgerProtocolError::InvalidOperation)
            }
            Self::Retain { cutoff }
                if cutoff.is_empty() || cutoff.len() > MAX_TIMESTAMP_BYTES || cutoff.chars().any(char::is_control) =>
            {
                return Err(LedgerProtocolError::InvalidOperation)
            }
            Self::Snapshot { snapshot_id } if snapshot_id.iter().all(|byte| *byte == 0) => {
                return Err(LedgerProtocolError::InvalidOperation)
            }
            _ => {}
        }
        let encoded = rmp_serde::to_vec_named(self).map_err(|_| LedgerProtocolError::InvalidOperation)?;
        if encoded.len() > MAX_LEDGER_OPERATION_BYTES {
            return Err(LedgerProtocolError::OperationTooLarge);
        }
        Ok(())
    }
}

fn valid_event_id(value: &str) -> bool {
    value.len() == MAX_EVENT_ID_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Named read intents. SQL stays inside `capsem-logger`; clients can select
/// filters and bounds, but cannot submit statements.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LedgerQuery {
    SecurityLatest {
        limit: u16,
        detection_only: bool,
    },
    Timeline {
        layers: Vec<LedgerTimelineLayer>,
        cutoff: String,
        trace_id: Option<String>,
        limit: u16,
    },
    History {
        layers: Vec<LedgerHistoryLayer>,
        search: Option<String>,
        limit: u16,
        offset: u64,
    },
    StatsDetail,
    Triage {
        limit: u16,
    },
}

impl LedgerQuery {
    pub fn validate(&self) -> Result<(), LedgerProtocolError> {
        let valid_limit = |limit: u16| limit > 0 && usize::from(limit) <= MAX_LEDGER_QUERY_ROWS;
        let bounded_text = |value: &str| value.len() <= MAX_QUERY_TEXT_BYTES;
        let valid = match self {
            Self::SecurityLatest { limit, .. } | Self::Triage { limit } => valid_limit(*limit),
            Self::Timeline {
                layers,
                cutoff,
                trace_id,
                limit,
            } => {
                !layers.is_empty()
                    && valid_limit(*limit)
                    && cutoff.len() <= MAX_TIMESTAMP_BYTES
                    && !cutoff.chars().any(char::is_control)
                    && trace_id.as_deref().is_none_or(bounded_text)
            }
            Self::History {
                layers, search, limit, ..
            } => !layers.is_empty() && valid_limit(*limit) && search.as_deref().is_none_or(bounded_text),
            Self::StatsDetail => true,
        };
        if valid {
            Ok(())
        } else {
            Err(LedgerProtocolError::InvalidOperation)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LedgerTimelineLayer {
    Exec,
    Tool,
    Net,
    File,
    Model,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LedgerHistoryLayer {
    Exec,
    Audit,
}

/// SQLite values leave the worker as an explicit Rust value, not JSON text.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LedgerValue {
    Null,
    Integer(i64),
    Real(f64),
    Text(String),
    Blob(#[serde(with = "serde_bytes")] Vec<u8>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LedgerRows {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<LedgerValue>>,
}

impl LedgerRows {
    pub fn validate(&self) -> Result<(), LedgerProtocolError> {
        let columns = self.columns.len();
        let valid = columns <= MAX_LEDGER_COLUMNS
            && self.rows.len() <= MAX_LEDGER_QUERY_ROWS
            && self
                .columns
                .iter()
                .all(|column| !column.is_empty() && column.len() <= MAX_LEDGER_COLUMN_BYTES)
            && self.rows.iter().all(|row| {
                row.len() == columns
                    && row.iter().all(|value| match value {
                        LedgerValue::Text(value) => value.len() <= MAX_LEDGER_CELL_BYTES,
                        LedgerValue::Blob(value) => value.len() <= MAX_LEDGER_CELL_BYTES,
                        LedgerValue::Null | LedgerValue::Integer(_) | LedgerValue::Real(_) => true,
                    })
            });
        if valid {
            Ok(())
        } else {
            Err(LedgerProtocolError::InvalidOperation)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerBodyMetadata {
    pub event_id: String,
    pub source_table: String,
    pub direction: BodyDirection,
    pub content_type: Option<String>,
    pub original_bytes: u64,
    pub stored_bytes: u64,
    pub truncated: bool,
    pub body_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerExportSummary {
    pub records: u64,
    pub bytes_written: u64,
    pub skipped_count: u64,
    pub skipped_by_reason: BTreeMap<String, u64>,
}

/// A request may yield several replies with its same request id. Body and
/// WARC bytes are always binary chunks below the frame limit.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LedgerReply {
    Accepted {
        admission_id: u64,
    },
    Durable {
        through_admission_id: u64,
    },
    Query {
        sets: Vec<LedgerRows>,
    },
    Counters {
        counters: Box<LedgerCounters>,
    },
    BodyStart {
        body_id: u32,
        metadata: LedgerBodyMetadata,
    },
    BodyChunk {
        body_id: u32,
        offset: u64,
        #[serde(with = "serde_bytes")]
        bytes: Vec<u8>,
    },
    BodiesComplete,
    Retained {
        outcome: RetainOutcome,
    },
    WarcChunk {
        offset: u64,
        #[serde(with = "serde_bytes")]
        bytes: Vec<u8>,
    },
    WarcComplete {
        summary: LedgerExportSummary,
    },
    Snapshotted,
    ShuttingDown,
}

impl LedgerReply {
    pub fn validate(&self) -> Result<(), LedgerProtocolError> {
        let valid = match self {
            Self::Accepted { admission_id } => *admission_id != 0,
            Self::Durable { .. } | Self::Counters { .. } | Self::BodiesComplete | Self::Retained { .. } => true,
            Self::Query { sets } => {
                sets.len() <= MAX_LEDGER_RESULT_SETS && sets.iter().all(|rows| rows.validate().is_ok())
            }
            Self::BodyStart { body_id, metadata } => {
                *body_id != 0
                    && valid_event_id(&metadata.event_id)
                    && !metadata.source_table.is_empty()
                    && metadata.source_table.len() <= MAX_LEDGER_COLUMN_BYTES
                    && metadata
                        .content_type
                        .as_deref()
                        .is_none_or(|value| value.len() <= MAX_LEDGER_COLUMN_BYTES)
                    && metadata.body_hash.len() <= MAX_LEDGER_COLUMN_BYTES
            }
            Self::BodyChunk { body_id, bytes, .. } => {
                *body_id != 0 && !bytes.is_empty() && bytes.len() <= MAX_LEDGER_STREAM_CHUNK_BYTES
            }
            Self::WarcChunk { bytes, .. } => !bytes.is_empty() && bytes.len() <= MAX_LEDGER_STREAM_CHUNK_BYTES,
            Self::WarcComplete { summary } => {
                summary.skipped_by_reason.len() <= 32
                    && summary
                        .skipped_by_reason
                        .keys()
                        .all(|reason| !reason.is_empty() && reason.len() <= MAX_LEDGER_COLUMN_BYTES)
            }
            Self::Snapshotted | Self::ShuttingDown => true,
        };
        if valid {
            Ok(())
        } else {
            Err(LedgerProtocolError::InvalidOperation)
        }
    }
}

#[cfg(test)]
mod tests;
