//! Checkpoint the semantic parser's preceding character and synchronized output.

use serde::{Deserialize, Serialize};

use super::{
    Duration, Instant, Processor, ProcessorState, StdSyncHandler, SyncState, SYNC_BUFFER_SIZE, SYNC_UPDATE_TIMEOUT,
};
use crate::checkpoint::ParserCheckpoint;
use crate::Parser;

/// A complete standard processor continuation. Restore never invokes a handler.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessorCheckpoint {
    version: u16,
    parser: ParserCheckpoint,
    preceding_char: Option<char>,
    sync_buffer: Vec<u8>,
    sync_remaining_nanos: Option<u64>,
}

impl Processor<StdSyncHandler> {
    /// Copy parser continuation with a caller-owned bound on each pending buffer.
    pub fn checkpoint(&self, max_pending_bytes: usize) -> Result<ProcessorCheckpoint, &'static str> {
        if self.state.sync_state.buffer.len() > max_pending_bytes {
            return Err("processor checkpoint exceeds pending-byte budget");
        }
        Ok(ProcessorCheckpoint {
            version: 1,
            parser: self.parser.checkpoint(max_pending_bytes)?,
            preceding_char: self.state.preceding_char,
            sync_buffer: self.state.sync_state.buffer.clone(),
            sync_remaining_nanos: self
                .state
                .sync_state
                .timeout
                .timeout
                .map(|deadline| deadline.saturating_duration_since(Instant::now()).as_nanos() as u64),
        })
    }

    /// Restore relative timeout duration; suspended wall time does not consume it.
    pub fn from_checkpoint(state: ProcessorCheckpoint, max_pending_bytes: usize) -> Result<Self, &'static str> {
        if state.version != 1
            || state.sync_buffer.len() > max_pending_bytes
            || state.sync_buffer.len() >= SYNC_BUFFER_SIZE - 1
            || matches!(state.sync_remaining_nanos, Some(nanos) if u128::from(nanos) > SYNC_UPDATE_TIMEOUT.as_nanos())
            || (!state.sync_buffer.is_empty() && state.sync_remaining_nanos.is_none())
        {
            return Err("invalid processor checkpoint");
        }
        Ok(Self {
            parser: Parser::from_checkpoint(state.parser, max_pending_bytes)?,
            state: ProcessorState {
                preceding_char: state.preceding_char,
                sync_state: SyncState {
                    buffer: state.sync_buffer,
                    timeout: StdSyncHandler {
                        timeout: state
                            .sync_remaining_nanos
                            .map(|nanos| Instant::now() + Duration::from_nanos(nanos)),
                    },
                },
            },
        })
    }
}
