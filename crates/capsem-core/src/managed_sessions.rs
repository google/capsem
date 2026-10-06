//! Capability ownership and durable admission precede service VM effects.

use std::{
    fmt,
    io::{Read, Write},
    path::Path,
};

use anyhow::{ensure, Context, Result};
use capsem_foundation::unix::{
    contained::{ContainedDir, ContainedOpenOptions},
    fs::ensure_private_dir,
    lock::{try_acquire, FileLock, LockAttempt, LockMode},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// An opaque 256-bit ownership secret. It is never serialized or logged.
#[derive(Clone)]
pub struct Capability([u8; 32]);

impl Capability {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    fn hash(&self, request: Uuid) -> blake3::Hash {
        let mut hash = blake3::Hasher::new_derive_key("capsem managed session capability v1");
        hash.update(request.as_bytes());
        hash.update(&self.0);
        hash.finalize()
    }
}

impl fmt::Debug for Capability {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Capability([REDACTED])")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Reserved,
    Creating,
    Active,
    Closing,
    Closed,
    Unknown,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    schema_version: u32,
    request: Uuid,
    generation: Uuid,
    capability_hash: [u8; 32],
    state: State,
    #[serde(default)]
    vm: Option<VmBinding>,
}

/// A service-proven VM identity. Generation must identify the actual spawn,
/// rather than a recyclable PID or a user-visible VM name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VmBinding {
    id: String,
    generation: Uuid,
}

impl VmBinding {
    pub fn new(id: String, generation: Uuid) -> Result<Self> {
        ensure!(
            !id.is_empty() && id.len() <= 256 && !id.chars().any(char::is_control),
            "invalid managed VM identity"
        );
        ensure!(!generation.is_nil(), "managed VM generation is nil");
        Ok(Self { id, generation })
    }
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn generation(&self) -> Uuid {
        self.generation
    }
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    request: Uuid,
    generation: Uuid,
    state: State,
    vm: Option<VmBinding>,
}

impl Snapshot {
    pub fn request(&self) -> Uuid {
        self.request
    }
    pub fn generation(&self) -> Uuid {
        self.generation
    }
    pub fn state(&self) -> State {
        self.state
    }
    pub fn vm(&self) -> Option<&VmBinding> {
        self.vm.as_ref()
    }
}

impl From<&Record> for Snapshot {
    fn from(record: &Record) -> Self {
        Self {
            request: record.request,
            generation: record.generation,
            state: record.state,
            vm: record.vm.clone(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Ticket {
    request: Uuid,
    generation: Uuid,
}

impl Ticket {
    pub fn request(&self) -> Uuid {
        self.request
    }
    pub fn generation(&self) -> Uuid {
        self.generation
    }
}

#[derive(Debug)]
pub enum Reservation {
    New(Ticket),
    Existing(Snapshot),
}

/// Synchronous private-store primitives: service callers use their blocking
/// execution boundary. Contention is immediate refusal, never a machine lock.
pub struct Registry {
    root: ContainedDir,
}

impl Registry {
    pub fn open(root: &Path) -> Result<Self> {
        ensure_private_dir(root)?;
        let root = ContainedDir::open_root(root)?;
        root.validate_private()?;
        Ok(Self { root })
    }

    pub fn reserve(&self, request: Uuid, capability: &Capability) -> Result<Reservation> {
        ensure!(!request.is_nil(), "managed request identity is nil");
        let _lease = self.lease()?;
        if let Some(record) = self.read(request)? {
            // blake3::Hash equality is constant-time for these 32-byte hashes.
            ensure!(
                blake3::Hash::from(record.capability_hash) == capability.hash(request),
                "managed capability mismatch"
            );
            return Ok(Reservation::Existing(Snapshot::from(&record)));
        }
        let record = Record {
            schema_version: 1,
            request,
            generation: Uuid::new_v4(),
            capability_hash: *capability.hash(request).as_bytes(),
            state: State::Reserved,
            vm: None,
        };
        self.write(&record)?;
        Ok(Reservation::New(Ticket {
            request,
            generation: record.generation,
        }))
    }

    fn lease(&self) -> Result<FileLock> {
        match try_acquire(&self.root.path().join("registry.lock"), LockMode::Exclusive)? {
            LockAttempt::Acquired(lease) => Ok(lease),
            LockAttempt::Contended => anyhow::bail!("managed ownership store is busy"),
        }
    }

    fn ticket_record(&self, ticket: &Ticket) -> Result<Record> {
        let record = self.read(ticket.request)?.context("managed reservation is missing")?;
        ensure!(
            record.generation == ticket.generation,
            "managed reservation generation mismatch"
        );
        Ok(record)
    }

    fn read(&self, request: Uuid) -> Result<Option<Record>> {
        let name = format!("{request}.json");
        let file = match self.root.open_file(name.as_ref(), ContainedOpenOptions::read_only()) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        // Fixed-shape ownership metadata contains no user payload.
        const RECORD_LIMIT: u64 = 4096;
        ensure!(
            file.metadata()?.len() <= RECORD_LIMIT,
            "managed ownership record exceeds fixed-shape limit"
        );
        let mut bytes = Vec::new();
        file.take(RECORD_LIMIT + 1).read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() as u64 <= RECORD_LIMIT,
            "managed ownership record grew during read"
        );
        let record: Record = serde_json::from_slice(&bytes).context("invalid managed ownership record")?;
        ensure!(
            record.schema_version == 1 && record.request == request && !record.generation.is_nil(),
            "managed ownership identity mismatch"
        );
        if let Some(vm) = &record.vm {
            VmBinding::new(vm.id.clone(), vm.generation)?;
        }
        ensure!(
            match record.state {
                State::Reserved | State::Creating | State::Closed => record.vm.is_none(),
                State::Active => record.vm.is_some(),
                State::Closing | State::Unknown => true,
            },
            "managed ownership state and VM binding disagree"
        );
        Ok(Some(record))
    }

    fn write(&self, record: &Record) -> Result<()> {
        let bytes = serde_json::to_vec(record)?;
        let temporary_name = format!(".pending-managed-{}", Uuid::new_v4());
        let mut temporary = self.root.create_new_private_file(temporary_name.as_ref())?;
        let result = (|| -> Result<()> {
            temporary.write_all(&bytes)?;
            temporary.sync_all()?;
            self.root.rename_to(
                temporary_name.as_ref(),
                &self.root,
                format!("{}.json", record.request).as_ref(),
            )?;
            self.root.sync()?;
            Ok(())
        })();
        if result.is_err() {
            match self.root.remove_non_directory(temporary_name.as_ref()) {
                Ok(()) => self.root.sync()?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        result
    }
}

mod recovery;
mod transitions;

#[cfg(test)]
mod tests;
