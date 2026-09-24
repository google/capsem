//! Capture a coherent session ledger for the manual economics evidence owner.

use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args_os().skip(1).map(PathBuf::from);
    let source = args
        .next()
        .ok_or_else(|| anyhow::anyhow!("missing source session directory"))?;
    let destination = args
        .next()
        .ok_or_else(|| anyhow::anyhow!("missing destination session directory"))?;
    if args.next().is_some() {
        anyhow::bail!("expected exactly source and destination session directories");
    }
    capsem_logger::snapshot_session_ledger(&source, &destination)
}
