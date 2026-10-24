//! Authoritative OS account metadata, independent of inherited environment.
use std::io;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub uid: u32,
    pub home: PathBuf,
}

/// Resolve public OS account metadata. NSS/DirectoryServices may block; callers
/// must own the complete invocation deadline, not merely a later DB timeout.
pub fn by_name(name: &str) -> io::Result<Option<Account>> {
    if name.is_empty() || name.as_bytes().contains(&0) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid account name"));
    }
    nix::unistd::User::from_name(name)
        .map_err(super::errno::io)
        .map(|account| {
            account.map(|account| Account {
                uid: account.uid.as_raw(),
                home: account.dir,
            })
        })
}

#[cfg(test)]
mod tests;
