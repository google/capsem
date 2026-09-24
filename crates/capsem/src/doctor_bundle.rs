//! Getting `capsem doctor --bundle`'s tarball out of the guest share.

/// Copy the in-VM doctor bundle out of the guest share: `guest/`, or the
/// workspace. The guest wrote it, so nothing on the way is followed -- a link
/// planted in its place would copy a host file into a bundle the user shares.
pub(crate) fn copy_out(session_dir: &std::path::Path, dest: &std::path::Path) -> std::io::Result<u64> {
    use capsem_foundation::unix::contained::{ContainedDir, ContainedOpenOptions};
    let name = std::ffi::OsStr::new("doctor-bundle.tar");
    let in_share = ContainedDir::open_root(session_dir)?
        .descend(std::ffi::OsStr::new(capsem_core::GUEST_SHARE_DIR))
        .and_then(|share| share.open_file(name, ContainedOpenOptions::read_only()));
    let mut source = match in_share {
        Ok(file) => file,
        Err(_) => {
            capsem_core::session::open_workspace(session_dir)?.open_file(name, ContainedOpenOptions::read_only())?
        }
    };
    std::io::copy(&mut source, &mut std::fs::File::create(dest)?)
}

#[cfg(test)]
mod tests;
