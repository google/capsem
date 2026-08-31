//! Low-privilege MCP aggregator subprocess entrypoint.

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    capsem_mcp_aggregator::runtime::run().await
}
