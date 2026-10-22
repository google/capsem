//! Explicit parser continuation, without replaying terminal input.
//!
//! Checkpoints are versioned implementation data. Callers must bound their
//! serialized input before deserialization and choose a pending-byte budget.

use serde::{Deserialize, Serialize};

use crate::params::ParamsCheckpoint;
use crate::{Params, Parser, State, MAX_INTERMEDIATES, MAX_OSC_PARAMS};

/// All parser fields needed to resume an unfinished escape or UTF-8 scalar.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParserCheckpoint {
    version: u16,
    state: State,
    intermediates: [u8; MAX_INTERMEDIATES],
    intermediate_idx: usize,
    params: ParamsCheckpoint,
    param: u16,
    osc_raw: Vec<u8>,
    osc_params: [(usize, usize); MAX_OSC_PARAMS],
    osc_num_params: usize,
    ignoring: bool,
    partial_utf8: [u8; 4],
    partial_utf8_len: usize,
}

impl Parser {
    /// Copy state after checking its retained OSC buffer against the caller's budget.
    pub fn checkpoint(&self, max_pending_bytes: usize) -> Result<ParserCheckpoint, &'static str> {
        if self.osc_raw.len() > max_pending_bytes {
            return Err("parser checkpoint exceeds pending-byte budget");
        }
        Ok(ParserCheckpoint {
            version: 1,
            state: self.state,
            intermediates: self.intermediates,
            intermediate_idx: self.intermediate_idx,
            params: self.params.checkpoint(),
            param: self.param,
            osc_raw: self.osc_raw.clone(),
            osc_params: self.osc_params,
            osc_num_params: self.osc_num_params,
            ignoring: self.ignoring,
            partial_utf8: self.partial_utf8,
            partial_utf8_len: self.partial_utf8_len,
        })
    }

    /// Restore only validated state; no performer is called and no bytes are parsed.
    pub fn from_checkpoint(state: ParserCheckpoint, max_pending_bytes: usize) -> Result<Self, &'static str> {
        if state.version != 1
            || state.intermediate_idx > MAX_INTERMEDIATES
            || state.osc_num_params > MAX_OSC_PARAMS
            || state.osc_raw.len() > max_pending_bytes
            || state.partial_utf8_len >= state.partial_utf8.len()
        {
            return Err("invalid parser checkpoint");
        }
        let mut end = 0;
        for &(start, next_end) in &state.osc_params[..state.osc_num_params] {
            if start != end || next_end < start || next_end > state.osc_raw.len() {
                return Err("invalid parser checkpoint OSC offsets");
            }
            end = next_end;
        }
        if state.partial_utf8_len != 0 {
            match core::str::from_utf8(&state.partial_utf8[..state.partial_utf8_len]) {
                Err(error) if error.valid_up_to() == 0 && error.error_len().is_none() => (),
                _ => return Err("invalid parser checkpoint UTF-8 continuation"),
            }
        }
        let params = Params::from_checkpoint(state.params).ok_or("invalid parser checkpoint parameters")?;
        Ok(Self {
            state: state.state,
            intermediates: state.intermediates,
            intermediate_idx: state.intermediate_idx,
            params,
            param: state.param,
            osc_raw: state.osc_raw,
            osc_params: state.osc_params,
            osc_num_params: state.osc_num_params,
            ignoring: state.ignoring,
            partial_utf8: state.partial_utf8,
            partial_utf8_len: state.partial_utf8_len,
        })
    }
}
