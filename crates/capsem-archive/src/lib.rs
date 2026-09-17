//! Append-only, block-compressed body archive for one Capsem session.
//!
//! SQLite stays the index of every ledger row. Bodies (HTTP, model, tool,
//! security payloads) are staged into a block, deflated together when the
//! block seals, and appended to `session.bodies`. Consecutive bodies from the
//! same provider share most of their text, so a block compresses about
//! twice as well as a body on its own (11x vs 6x measured), and the raw
//! bytes leave RAM the moment the block seals.
//!
//! Pure Rust by rule: the runtime links one C library (SQLite) and this
//! crate must not add a second in front of attacker-influenced bytes.
//! `tests/citadel/test_runtime_native_dependencies.py` holds it.

pub mod format;
pub mod reader;
pub mod retain;
pub mod warc;
pub mod writer;

pub use format::{BodyRef, BLOCK_HEADER_BYTES, FILE_HEADER_BYTES, TARGET_BLOCK_BYTES};
pub use reader::BodyLogReader;
pub use writer::{BodyLogWriter, EncodedBlock, PendingBlock, SealedBlock};

#[derive(Debug, thiserror::Error)]
pub enum ArchiveError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("archive path is a symlink: {0}")]
    Symlink(std::path::PathBuf),
    #[error("bad archive header")]
    BadFileHeader,
    #[error("bad block header at offset {0}")]
    BadBlockHeader(u64),
    #[error("block at offset {0} failed integrity check")]
    Integrity(u64),
    #[error("block at offset {0} did not inflate: {1}")]
    Inflate(u64, String),
    #[error("body reference out of range for its block")]
    RefOutOfRange,
}

pub type Result<T> = std::result::Result<T, ArchiveError>;
