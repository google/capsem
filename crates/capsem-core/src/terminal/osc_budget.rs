//! Bound OSC before the upstream parser buffers it. Other VT bytes pass through
//! unchanged; this owns a byte budget, not terminal interpretation.
use capsem_proto::TERMINAL_CONTROL_STRING_BYTES;
use serde::{Deserialize, Serialize};

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct OscBudget {
    escape: bool,
    osc: bool,
    discarded: bool,
    pending: Vec<u8>,
}

impl OscBudget {
    pub fn valid_checkpoint(&self) -> bool {
        self.pending.len() <= TERMINAL_CONTROL_STRING_BYTES
            && !(self.escape && self.osc)
            && if self.discarded {
                self.osc && self.pending.is_empty()
            } else if self.osc {
                self.pending.first() == Some(&b']')
                    && !self.pending.iter().any(|byte| matches!(byte, 7 | 0x18 | 0x1a | 0x1b))
            } else {
                self.pending.is_empty()
            }
    }

    pub fn process(&mut self, mut bytes: &[u8], mut emit: impl FnMut(&[u8])) {
        while !bytes.is_empty() {
            if self.osc {
                let end = bytes.iter().position(|b| matches!(b, 7 | 0x18 | 0x1a | 0x1b));
                let count = end.unwrap_or(bytes.len());
                if !self.discarded {
                    if count > TERMINAL_CONTROL_STRING_BYTES.saturating_sub(self.pending.len()) {
                        self.pending.clear();
                        self.discarded = true;
                        emit(b"\x18"); // Cancel the ESC held at the upstream parser.
                    } else {
                        self.pending.extend_from_slice(&bytes[..count]);
                    }
                }
                bytes = &bytes[count..];
                let Some(&terminator) = bytes.first() else { break };
                let discarded = self.discarded;
                if !discarded {
                    emit(&self.pending);
                }
                self.pending.clear();
                self.osc = false;
                self.discarded = false;
                // ESC ends OSC and starts a new escape sequence. BEL/CAN/SUB
                // terminate OSC; a discarded OSC must not become an audible bell.
                if terminator == 0x1b {
                    emit(&bytes[..1]);
                    self.escape = true;
                } else if !discarded {
                    emit(&bytes[..1]);
                }
                bytes = &bytes[1..];
            } else if self.escape {
                let byte = bytes[0];
                bytes = &bytes[1..];
                if byte == b']' {
                    self.osc = true;
                    self.pending.push(byte);
                    self.escape = false;
                } else {
                    emit(&[byte]);
                    self.escape = matches!(byte, 0..=0x17 | 0x19 | 0x1b..=0x1f | 0x7f..=0xff);
                }
            } else {
                let count = bytes.iter().position(|b| *b == 0x1b).map_or(bytes.len(), |i| i + 1);
                emit(&bytes[..count]);
                self.escape = bytes[count - 1] == 0x1b;
                bytes = &bytes[count..];
            }
        }
    }
}

#[cfg(test)]
mod tests;
