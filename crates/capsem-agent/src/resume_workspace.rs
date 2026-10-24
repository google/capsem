//! One bounded directory read verifies the resumed workspace transport.
use std::io;
use std::path::Path;

pub(crate) fn enumerate(path: &Path) -> io::Result<()> {
    first_entry(std::fs::read_dir(path)?)
}

fn first_entry<T>(mut entries: impl Iterator<Item = io::Result<T>>) -> io::Result<()> {
    // One READDIR result is the existing bounded transport probe. An empty
    // successful read is healthy; an iteration error is not an entry.
    entries.next().transpose().map(|_| ())
}

#[cfg(test)]
mod tests;
