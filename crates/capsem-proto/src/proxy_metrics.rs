//! Bounded OTLP relay messages for a confined proxy worker.
//!
//! The worker submits only an already encoded metrics body. Collector
//! selection, credentials and transport remain coordinator authority.

use serde::{Deserialize, Serialize};

/// Same payload ceiling as the service HTTP relay.
pub const MAX_PROXY_METRIC_BODY_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxyMetricRequest {
    #[serde(with = "serde_bytes")]
    pub body: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyMetricBrokerMessage {
    Hello { session_id: String },
    Response(ProxyMetricResponse),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyMetricResponse {
    Relayed {
        status: u16,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        content_type: Option<String>,
        #[serde(with = "serde_bytes")]
        body: Vec<u8>,
    },
    Rejected {
        message: String,
    },
}

#[cfg(test)]
mod tests;
