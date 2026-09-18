//! Behaviour that spans the writer, the reader and the file between them:
//! crash at every boundary, a reader running beside the writer, and the
//! stream properties the format rests on. Helpers shared by every v2 test
//! module live here.

use std::path::{Path, PathBuf};

mod concurrent;
mod crash;
mod stream;

pub(crate) fn archive(dir: &tempfile::TempDir) -> PathBuf {
    dir.path().join("session.bodies")
}

pub(crate) fn file_len(path: &Path) -> u64 {
    std::fs::metadata(path).unwrap().len()
}

/// Deterministic pseudo-random bytes: the same corpus on every machine, so a
/// failure is reproducible from the test name alone.
pub(crate) struct XorShift(pub(crate) u64);

impl XorShift {
    pub(crate) fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    pub(crate) fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }

    /// Incompressible bytes.
    pub(crate) fn bytes(&mut self, len: usize) -> Vec<u8> {
        (0..len).map(|_| (self.next() & 0xff) as u8).collect()
    }

    /// JSON-shaped text from a small vocabulary: what a model exchange looks
    /// like to deflate, and what gives a shared dictionary something to find.
    pub(crate) fn text(&mut self, len: usize) -> Vec<u8> {
        const WORDS: [&str; 12] = [
            "{\"role\":\"assistant\",",
            "\"content\":",
            "\"tool_use\"",
            "\"input\":{",
            "the",
            "file",
            "result",
            "}",
            "\"id\":\"toolu_",
            "stream",
            "delta",
            ",",
        ];
        let mut out = Vec::with_capacity(len + 32);
        while out.len() < len {
            out.extend_from_slice(WORDS[self.below(WORDS.len())].as_bytes());
            if self.below(5) == 0 {
                out.extend_from_slice(format!("{:x}", self.next() >> 44).as_bytes());
            }
            out.push(b' ');
        }
        out.truncate(len);
        out
    }
}
