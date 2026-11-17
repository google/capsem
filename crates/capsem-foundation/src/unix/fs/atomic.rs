//! Private publication retains the caller's selected directory descriptor.
use super::*;
use crate::unix::contained::{check_component, ContainedDir};

struct Temporary<'a> {
    directory: &'a ContainedDir,
    name: OsString,
    armed: bool,
}
impl Drop for Temporary<'_> {
    fn drop(&mut self) {
        if self.armed {
            let _ = self.directory.remove_private_file(&self.name);
        }
    }
}

/// Atomically replace one private regular file below a private directory.
/// Sync the complete file before rename, then sync the same directory handle.
/// A replaced pathname above it cannot redirect publication or durability.
/// As with `atomic_write_private`, a directory-sync error may follow a visible
/// rename: callers must reconcile that outcome before committing other state.
pub fn atomic_write_private_at(directory: &ContainedDir, name: &OsStr, bytes: &[u8]) -> io::Result<()> {
    check_component(name)?;
    directory.validate_private()?;
    let (mut file, mut temporary) = loop {
        let temporary_name = temporary_name(name);
        match directory.create_new_private_file(&temporary_name) {
            Ok(file) => {
                break (
                    file,
                    Temporary {
                        directory,
                        name: temporary_name,
                        armed: true,
                    },
                )
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    };
    file.write_all(bytes)?;
    file.sync_all()?;
    directory.rename_to(&temporary.name, directory, name)?;
    temporary.armed = false;
    directory.sync()
}
