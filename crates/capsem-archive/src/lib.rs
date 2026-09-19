//! Append-only, block-compressed body archive for one Capsem session.
//!
//! SQLite stays the index of every ledger row. Bodies (HTTP, model, tool,
//! security payloads) are fed to one open deflate stream, a block, and each
//! disk flush appends what that stream produced as a segment of the block in
//! `session.bodies`. The block stays open across flushes, so every body
//! compresses against the ones before it, until it reaches its target size.
//! Consecutive bodies from the same provider share most of their text, so a
//! block compresses several times better than a body on its own.
//!
//! The crate root is the current format, version 2 (`v2`). The version 1
//! modules (`format`, `writer`, `reader`, `retain`) remain only until nothing
//! reads a version 1 archive.
//!
//! Pure Rust by rule: the runtime links one C library (SQLite) and this
//! crate must not add a second in front of attacker-influenced bytes.
//! `tests/citadel/test_runtime_native_dependencies.py` holds it.

#[cfg(not(unix))]
compile_error!("capsem-archive relies on O_NOFOLLOW and mode 0600");

pub mod format;
pub mod reader;
pub mod retain;
pub mod v2;
pub mod warc;
pub mod writer;

pub use v2::{
    commit_retained, stage_retained_blocks, BodyLogReader, BodyLogWriter, BodyRef, RetainedStaging, SegmentWritten,
    BLOCK_HEADER_BYTES, FILE_HEADER_BYTES, MAX_BLOCK_RAW_BYTES, TARGET_BLOCK_BYTES,
};
pub use warc::{write_record, WarcRecord};

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
    /// A block written with a codec, flag or reserved bit this reader does
    /// not know. Refused by name: inflating it as deflate would be reading a
    /// later writer's bytes as something they are not.
    #[error("block at offset {block_offset} uses unsupported codec {codec}")]
    UnsupportedCodec { block_offset: u64, codec: u8 },
    /// A segment header that is not one, or that fails its bounds: wrong
    /// magic or flags, a raw extent that does not continue the block, a
    /// length past the ceilings, or an extent that does not end on a segment.
    #[error("bad segment at offset {0}")]
    BadSegment(u64),
    #[error("block at offset {0} failed integrity check")]
    Integrity(u64),
    #[error("block at offset {0} did not inflate: {1}")]
    Inflate(u64, String),
    #[error("block at offset {0} is truncated: the file ends inside its payload")]
    TruncatedBlock(u64),
    #[error("body reference out of range for its block")]
    RefOutOfRange,
    /// A write failed part-way through a block, so the file's end is no
    /// longer where the writer believes it is. Every later offset would be a
    /// guess, so the writer refuses all further work instead of handing out
    /// references nobody can resolve.
    #[error("archive writer is poisoned by an earlier partial write")]
    Poisoned,
    #[error("body of {len} bytes exceeds the {max}-byte block ceiling")]
    BodyTooLarge { len: usize, max: usize },
    /// The pending block cannot hold this body. Seal and stage it again; the
    /// body is guaranteed to fit a block of its own.
    #[error("pending block is full; seal it and stage this body again")]
    BlockFull,
    #[error("block {got} appended out of order; expected {expected}")]
    OutOfOrderBlock { expected: u64, got: u64 },
    /// A WARC header value carried a line break. Header blocks are
    /// line-oriented, so the rest of that value would have been read as
    /// headers of its own -- a forged record rather than a malformed one.
    #[error("WARC header field {field} contains a line break")]
    WarcHeaderBreak { field: &'static str },
    /// A record that captures something was given no `WARC-Target-URI`. The
    /// spec makes the header mandatory for every type but `warcinfo`, and a
    /// `resource` record without it describes nothing.
    #[error("a WARC {record_type} record must carry a target URI")]
    WarcMissingTargetUri { record_type: String },
}

pub type Result<T> = std::result::Result<T, ArchiveError>;
