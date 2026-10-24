//! Conservative default installed-home fence, not a saved-state converter.
use std::ffi::OsStr;
use std::io;
use std::path::Path;
use std::time::Instant;

use anyhow::{ensure, Context, Result};
use capsem_foundation::unix::{contained::ContainedDir, process};
use capsem_logger::host_admission::HostSchemaAdmission;

/// Only an already admitted current HOST ledger can use an in-service update.
/// An absent HOST ledger permits no retained default session namespace. This
/// observes names without reading legacy registry/session data or creating any
/// state. Installers must repeat admission after draining the old producers:
/// a single observation while they are live cannot guarantee atomic absence.
/// Custom run roots and intentional same-account namespace races are not a
/// compatibility guarantee of this fixed installed-home admission.
pub(super) fn admit(home: &Path, host: HostSchemaAdmission, require_current: bool, deadline: Instant) -> Result<()> {
    ensure!(Instant::now() < deadline, "preinstall admission deadline exceeded");
    if host == HostSchemaAdmission::Current {
        return Ok(());
    }
    ensure!(
        !require_current,
        "legacy installation requires drained producers before activation"
    );
    let uid = process::current_uid();
    let home = ContainedDir::open_root_no_follow(home)?;
    home.validate_owner_no_external_write(uid)?;
    let Some(capsem) = descend(&home, ".capsem", uid)? else {
        return Ok(());
    };
    let Some(run) = descend(&capsem, "run", uid)? else {
        return Ok(());
    };
    ensure!(
        run.entry_kind(OsStr::new("persistent_registry.json"))?.is_none(),
        "retained or unknown persistent registry requires compatible activation"
    );
    for name in ["instances", "sessions", "persistent"] {
        ensure!(Instant::now() < deadline, "preinstall admission deadline exceeded");
        if let Some(directory) = descend(&run, name, uid)? {
            ensure!(
                directory.is_empty()?,
                "retained {name} state requires compatible activation"
            );
        }
    }
    ensure!(Instant::now() < deadline, "preinstall admission deadline exceeded");
    Ok(())
}

fn descend(parent: &ContainedDir, name: &str, uid: u32) -> Result<Option<ContainedDir>> {
    match parent.descend(OsStr::new(name)) {
        Ok(directory) => {
            directory.validate_owner_no_external_write(uid)?;
            Ok(Some(directory))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("inspect installed {name} state")),
    }
}

#[cfg(test)]
mod tests;
