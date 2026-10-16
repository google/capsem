//! Installed local metadata is never paired with a caller-supplied URL.

use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use capsem_foundation::unix::{fs::open_regular_file_no_follow, process::current_uid};

use crate::{Error, Result};

const MESSAGE: &str = "cannot read local Capsem gateway metadata; Start Capsem and try again; check CAPSEM_RUN_DIR or CAPSEM_HOME if customized";

fn read(path: &Path, private: bool) -> std::io::Result<String> {
    let file = open_regular_file_no_follow(path)?;
    let info = file.metadata()?;
    if info.uid() != current_uid() || (private && info.mode() & 0o077 != 0) {
        return Err(std::io::Error::other("metadata ownership or permissions"));
    }
    let mut bytes = Vec::new();
    file.take(4097).read_to_end(&mut bytes)?;
    if bytes.len() > 4096 || !bytes.is_ascii() {
        return Err(std::io::Error::other("invalid metadata"));
    }
    String::from_utf8(bytes)
        .map(|text| text.trim().to_owned())
        .map_err(std::io::Error::other)
}

pub(crate) fn connection(root: &Path) -> Result<(String, String)> {
    let token = read(&root.join("gateway.token"), true).map_err(|_| Error::InvalidInput(MESSAGE))?;
    let port = read(&root.join("gateway.port"), false).map_err(|_| Error::InvalidInput(MESSAGE))?;
    let parsed = port.parse::<u16>().map_err(|_| Error::InvalidInput(MESSAGE))?;
    if parsed == 0
        || !port.bytes().all(|byte| byte.is_ascii_digit())
        || token.is_empty()
        || !token.bytes().all(|byte| (33..=126).contains(&byte))
    {
        return Err(Error::InvalidInput(MESSAGE));
    }
    Ok((format!("http://127.0.0.1:{parsed}"), token))
}
