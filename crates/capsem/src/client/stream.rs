//! CLI streams share SDK authentication and protocol handling.

use super::{gateway::gateway_error, GatewayClient};
use anyhow::Result;
use capsem_api::stream::StreamControl;
pub use capsem_sdk::transport::StreamEvent;
use capsem_sdk::transport::{CallOptions, GatewayStream};

impl GatewayClient {
    pub async fn open_stream(&self, id: &str, start: StreamControl) -> Result<GatewayStream> {
        let transport = self.transport().await?;
        Box::pin(transport.open_stream(
            id,
            start,
            CallOptions {
                timeout: Some(CallOptions::command_timeout(Some(110))),
            },
        ))
        .await
        .map_err(gateway_error)
    }
}

#[cfg(test)]
mod tests;
