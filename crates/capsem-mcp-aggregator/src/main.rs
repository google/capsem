//! Low-privilege MCP aggregator subprocess entrypoint.

use anyhow::Result;

fn main() -> Result<()> {
    let vm_id = std::env::var("CAPSEM_VM_ID").unwrap_or_else(|_| "unknown".into());
    let trace_id = std::env::var("CAPSEM_TRACE_ID").unwrap_or_else(|_| "unknown".into());
    // SAFETY: process entry, before runtime/telemetry threads or owned grants.
    unsafe {
        capsem_foundation::unix::process::clear_inherited_environment();
        capsem_foundation::unix::fd::close_inherited_descriptors()?;
    }
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(capsem_mcp_aggregator::runtime::run(vm_id, trace_id))
}
