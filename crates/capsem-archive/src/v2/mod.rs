//! Archive format version 2: blocks that stay open across disk flushes.
//!
//! Lives beside version 1 until the logger moves over, then replaces it.

pub mod format;
pub mod reader;
pub mod retain;
pub mod writer;

pub use format::{BodyRef, BLOCK_HEADER_BYTES, FILE_HEADER_BYTES, MAX_BLOCK_RAW_BYTES, TARGET_BLOCK_BYTES};
pub use reader::BodyLogReader;
pub use retain::{commit_retained, stage_retained_blocks, RetainedStaging};
pub use writer::{BodyLogWriter, SegmentWritten};

#[cfg(test)]
mod tests;
