//! Bounded metric records for a coordinator-granted gateway descriptor.
//!
//! This carries no collector URL, headers, response body or worker-chosen name.
//! The owning grant's epoch is fixed by Hello; request IDs start at one and
//! advance once per attempted export. Revocation belongs to the FD grant owner.
use serde::{Deserialize, Serialize};

pub const MAX_GATEWAY_METRIC_BODY_BYTES: usize = 4 * 1024 * 1024;
/// Binary body plus bounded named MessagePack metadata; checked before allocation.
pub const MAX_GATEWAY_METRIC_FRAME_BYTES: u32 = MAX_GATEWAY_METRIC_BODY_BYTES as u32 + 256;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayMetricRequest {
    #[serde(with = "serde_bytes")]
    pub generation: [u8; 16],
    pub request_id: u64,
    #[serde(with = "serde_bytes")]
    pub body: Vec<u8>,
}

impl GatewayMetricRequest {
    pub fn valid_for(&self, generation: [u8; 16], request_id: u64) -> bool {
        generation != [0; 16]
            && self.generation == generation
            && request_id != 0
            && self.request_id == request_id
            && !self.body.is_empty()
            && self.body.len() <= MAX_GATEWAY_METRIC_BODY_BYTES
    }
}

impl std::fmt::Debug for GatewayMetricRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GatewayMetricRequest")
            .field("request_id", &self.request_id)
            .field("body_bytes", &self.body.len())
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum GatewayMetricMessage {
    Hello {
        #[serde(with = "serde_bytes")]
        generation: [u8; 16],
    },
    Receipt {
        request_id: u64,
        outcome: GatewayMetricOutcome,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GatewayMetricOutcome {
    Accepted,
    Rejected,
    Unavailable,
}

#[cfg(test)]
mod tests;
