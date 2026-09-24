//! Tar entries of the support bundle.

use std::io::{Read, Write};
use std::time::SystemTime;

use anyhow::Result;
use tar::Builder as TarBuilder;

pub(super) fn add_bytes<W: Write>(tar: &mut TarBuilder<W>, path: &str, bytes: &[u8]) -> Result<()> {
    add_reader(tar, path, bytes.len() as u64, bytes)
}

/// Stream `len` bytes from `data` into the bundle, so a multi-GB image is
/// never held in memory.
pub(super) fn add_reader<W: Write>(tar: &mut TarBuilder<W>, path: &str, len: u64, data: impl Read) -> Result<()> {
    let mut header = tar::Header::new_gnu();
    header.set_size(len);
    header.set_mode(0o600);
    header.set_mtime(
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    );
    header.set_cksum();
    tar.append_data(&mut header, path, data)?;
    Ok(())
}
