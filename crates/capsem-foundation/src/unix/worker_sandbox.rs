//! OS confinement shared by host workers that retain bounded file authority.

use std::io;
use std::path::{Path, PathBuf};

/// The worker's post-startup process authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Role {
    Gateway,
    Ledger,
    VmOwner,
}

/// Filesystem access granted after confinement.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Access {
    ReadOnly,
    ReadWrite,
    Executable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PathRule {
    path: PathBuf,
    access: Access,
}

impl PathRule {
    pub fn new(path: impl Into<PathBuf>, access: Access) -> Self {
        Self {
            path: path.into(),
            access,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn access(&self) -> Access {
        self.access
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Policy {
    role: Role,
    paths: Vec<PathRule>,
}

impl Policy {
    pub fn new(role: Role) -> Self {
        Self {
            role,
            paths: Vec::new(),
        }
    }

    pub fn allow(mut self, path: impl Into<PathBuf>, access: Access) -> Self {
        self.paths.push(PathRule::new(path, access));
        self
    }

    pub fn role(&self) -> Role {
        self.role
    }

    pub fn paths(&self) -> &[PathRule] {
        &self.paths
    }
}

/// Restrict all existing and future threads before the worker publishes readiness.
pub fn confine(policy: &Policy) -> io::Result<()> {
    platform::confine(policy)
}

#[cfg(target_os = "linux")]
#[path = "worker_sandbox/linux.rs"]
mod platform;

#[cfg(target_os = "macos")]
#[path = "worker_sandbox/macos.rs"]
mod platform;

#[cfg(any(target_os = "macos", test))]
#[path = "worker_sandbox/seatbelt.rs"]
mod seatbelt;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod platform {
    use super::*;

    pub(super) fn confine(_policy: &Policy) -> io::Result<()> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "worker sandbox unavailable"))
    }
}

#[cfg(test)]
mod tests;
