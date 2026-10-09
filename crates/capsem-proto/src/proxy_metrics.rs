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
pub enum ProxyMetricResponse {
    Relayed {
        status: u16,
        content_type: Option<String>,
        #[serde(with = "serde_bytes")]
        body: Vec<u8>,
    },
    Rejected {
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_round_trip_binary_bodies_without_collector_authority() {
        let request = ProxyMetricRequest { body: vec![0, 0xff, 7] };
        let encoded = rmp_serde::to_vec_named(&request).unwrap();
        assert_eq!(rmp_serde::from_slice::<ProxyMetricRequest>(&encoded).unwrap(), request);

        let response = ProxyMetricResponse::Relayed {
            status: 202,
            content_type: Some("application/x-protobuf".into()),
            body: vec![9, 0, 8],
        };
        let encoded = rmp_serde::to_vec_named(&response).unwrap();
        assert_eq!(
            rmp_serde::from_slice::<ProxyMetricResponse>(&encoded).unwrap(),
            response
        );
    }
}
