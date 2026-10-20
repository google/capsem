//! Admit only the original service-issued process generation and retain its
//! lifetime fence before preparing any VM resources. The process entrypoint
//! arms the parent watcher after confinement so that thread cannot retain
//! ambient authority.
use crate::Args;
use anyhow::{ensure, Context, Result};
use capsem_core::managed_sessions::VmBinding;
use capsem_foundation::unix::contained::ContainedDir;
use std::path::Path;

pub(crate) fn enroll(args: &Args) -> Result<capsem_core::session::spawn_owner::SpawnOwner> {
    admit(
        &args.id,
        args.spawn_generation.as_deref(),
        args.parent_pid,
        &args.session_dir,
    )
}

fn admit(
    id: &str,
    generation: Option<&str>,
    parent: Option<u32>,
    directory: &Path,
) -> Result<capsem_core::session::spawn_owner::SpawnOwner> {
    let binding = validate(id, generation, parent, directory)?;
    capsem_core::session::spawn_owner::enter(directory, &binding, parent.context("VM service parent unavailable")?)
}

fn validate(id: &str, generation: Option<&str>, parent: Option<u32>, directory: &Path) -> Result<VmBinding> {
    ensure!(
        parent.is_some_and(capsem_guard::parent_is_expected),
        "VM service parent unavailable"
    );
    let generation = generation.context("VM spawn generation unavailable")?;
    let expected = VmBinding::new(
        id.to_owned(),
        generation.parse().context("invalid VM spawn generation")?,
    )?;
    ensure!(
        expected.generation().to_string() == generation,
        "noncanonical VM spawn generation"
    );
    let directory = ContainedDir::open_root(directory)?;
    ensure!(
        capsem_core::session::read_spawn_identity(&directory)?.as_ref() == Some(&expected),
        "VM trusted spawn identity changed"
    );
    Ok(expected)
}

#[cfg(test)]
mod tests;
