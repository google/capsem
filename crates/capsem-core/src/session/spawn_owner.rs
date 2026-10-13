//! A session's generation fence and pre-resource PID witness. The lifetime
//! lock covers boot admission and VM ownership; fencing is durable so delayed
//! children cannot enter after recovery releases it. A PID is a matching fact,
//! never an exit receipt: callers must separately prove original native exit.
use crate::managed_sessions::VmBinding;
use anyhow::{ensure, Context, Result};
use capsem_foundation::unix::{
    contained::ContainedDir,
    lock::{self, FileLock, LockAttempt, LockMode},
    process::{parent_process_id, probe, ProcessHandle, ProcessId, ProcessScope, ProcessState},
};
use serde::{Deserialize, Serialize};
use std::{io::Read, path::Path};

const RECORD: &str = "spawn-owner.json";
const LOCK: &str = "spawn-owner.lock";
const LIMIT: u64 = 4096;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    schema_version: u32,
    binding: VmBinding,
    parent_pid: u32,
    pid: Option<u32>,
    fenced: bool,
    scope: ProcessScope,
    retired: bool,
}

/// Retain before acquiring any VM resources and drop after their retirement.
pub struct SpawnOwner {
    _lock: FileLock,
}

/// Exclusive durable denial of this generation's future boot admission.
/// `pid()` still requires original native exit proof; acquiring this fence
/// alone cannot acknowledge an existing process as closed.
pub struct SpawnFence {
    _lock: FileLock,
    root: ContainedDir,
    record: Record,
}

impl SpawnFence {
    pub fn pid(&self) -> Option<ProcessId> {
        self.record
            .pid
            .map(|pid| ProcessId::try_from(pid).expect("validated native PID"))
    }
    pub fn parent(&self) -> ProcessId {
        ProcessId::try_from(self.record.parent_pid).expect("validated native parent")
    }

    /// Persist retirement only after the fenced preboot absence or a positive
    /// kernel exit observation in the original boot and PID namespace. A live
    /// replacement PID is never signalled and keeps the result unconfirmed.
    pub fn confirm_exit(&mut self) -> Result<bool> {
        if self.record.retired {
            return Ok(true);
        }
        ensure!(
            self.record.scope == ProcessScope::current()?,
            "original kernel process scope changed"
        );
        if let Some(pid) = self.pid() {
            if probe(pid)? != ProcessState::Gone {
                match ProcessHandle::open(pid) {
                    Ok(handle) if handle.has_exited()? => {}
                    Ok(_) => return Ok(false),
                    Err(error) => {
                        // The original may have exited between probe and pin.
                        // Preserve every error unless the kernel now confirms
                        // that this scoped PID no longer exists.
                        if probe(pid)? != ProcessState::Gone {
                            return Err(error.into());
                        }
                    }
                }
            }
        }
        self.record.retired = true;
        write(&self.root, &self.record)?;
        Ok(true)
    }
}

/// Prepare before native spawn. Existing process witnesses cannot be silently
/// overwritten; their original generation needs verified retirement first.
pub fn prepare(path: &Path, binding: &VmBinding, parent: u32) -> Result<()> {
    validate_binding(binding)?;
    validate_parent(parent)?;
    let _lock = acquire(path, true)?;
    let root = ContainedDir::open_root(path)?;
    if let Some(previous) = read(&root)? {
        ensure!(
            super::read_spawn_identity(&root)?.as_ref() == Some(&previous.binding),
            "previous spawn identity changed"
        );
        ensure!(
            previous.binding != *binding,
            "spawn generation cannot be prepared again"
        );
        ensure!(
            previous.fenced && previous.retired,
            "previous native process retirement is unconfirmed"
        );
    } else {
        ensure!(
            super::read_spawn_identity(&root)?.is_none(),
            "previous spawn owner protocol unavailable"
        );
    }
    let scope = ProcessScope::current()?;
    // Both publications happen under the same lifetime lock. Interrupted
    // publication is uncertainty and cannot authorize an unmatched boot.
    super::write_spawn_identity(&root, binding)?;
    write(
        &root,
        &Record {
            schema_version: 2,
            binding: binding.clone(),
            parent_pid: parent,
            pid: None,
            fenced: false,
            scope,
            retired: false,
        },
    )
}

pub fn enter(path: &Path, binding: &VmBinding, parent: u32) -> Result<SpawnOwner> {
    let lock = acquire(path, false)?;
    // Read the current record only after locking, never a pre-lock snapshot
    // which recovery could have fenced while admission waited.
    let root = ContainedDir::open_root(path)?;
    let mut record = matching(&root, binding)?;
    ensure!(
        record.scope == ProcessScope::current()?,
        "original kernel process scope changed"
    );
    ensure!(
        !record.fenced && record.pid.is_none(),
        "spawn generation boot admission retired or consumed"
    );
    ensure!(
        record.parent_pid == parent && parent_process_id().map(ProcessId::get) == Some(parent),
        "original native parent changed"
    );
    record.pid = Some(std::process::id());
    write(&root, &record)?;
    Ok(SpawnOwner { _lock: lock })
}

pub fn fence(path: &Path, binding: &VmBinding) -> Result<SpawnFence> {
    let lock = acquire(path, false)?;
    let root = ContainedDir::open_root(path)?;
    let mut record = matching(&root, binding)?;
    ensure!(
        record.retired || record.scope == ProcessScope::current()?,
        "original kernel process scope changed"
    );
    if !record.fenced {
        record.fenced = true;
        write(&root, &record)?;
    }
    Ok(SpawnFence {
        _lock: lock,
        root,
        record,
    })
}

fn acquire(path: &Path, create: bool) -> Result<FileLock> {
    let lock_path = path.join(LOCK);
    let attempt = if create {
        lock::try_acquire(&lock_path, LockMode::Exclusive)
    } else {
        lock::try_acquire_existing(&lock_path, LockMode::Exclusive)
    }?;
    match attempt {
        LockAttempt::Acquired(lock) => Ok(lock),
        LockAttempt::Contended => anyhow::bail!("original spawn lifetime is still owned"),
    }
}

fn matching(root: &ContainedDir, binding: &VmBinding) -> Result<Record> {
    validate_binding(binding)?;
    let record = read(root)?.context("original spawn owner protocol unavailable")?;
    ensure!(
        record.binding == *binding && super::read_spawn_identity(root)?.as_ref() == Some(binding),
        "original spawn generation changed"
    );
    Ok(record)
}

fn read(root: &ContainedDir) -> Result<Option<Record>> {
    // Reuse the Unix owner's single-link, private regular-file verification.
    // This read never appends or retains an old inode across publication.
    let file = match root.open_existing_private_append(RECORD.as_ref()) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    ensure!(
        file.metadata()?.len() <= LIMIT,
        "spawn owner record exceeds fixed limit"
    );
    let mut bytes = Vec::new();
    file.take(LIMIT + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= LIMIT, "spawn owner record grew during read");
    let record: Record = serde_json::from_slice(&bytes).context("invalid spawn owner record")?;
    ensure!(record.schema_version == 2, "unsupported spawn owner protocol");
    record.scope.validate()?;
    ensure!(!record.retired || record.fenced, "unfenced spawn retirement record");
    validate_binding(&record.binding)?;
    validate_parent(record.parent_pid)?;
    if let Some(pid) = record.pid {
        ProcessId::try_from(pid)?;
    }
    Ok(Some(record))
}

fn write(root: &ContainedDir, record: &Record) -> Result<()> {
    super::spawn_identity::publish(root, RECORD, &serde_json::to_vec(record)?)
}

fn validate_binding(binding: &VmBinding) -> Result<()> {
    VmBinding::new(binding.id().to_owned(), binding.generation())?;
    Ok(())
}

fn validate_parent(parent: u32) -> Result<()> {
    ProcessId::try_from(parent)?;
    ensure!(parent != 1, "original service parent cannot be init");
    Ok(())
}

#[cfg(test)]
mod tests;
