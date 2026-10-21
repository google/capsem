//! Append-only PTY transcript recording.
//!
//! capsem-process writes it and the service's transcript route reads it, so
//! the format lives here, in the shared library, with its one parser.
//! Records all terminal I/O (both input and output) with timestamps and
//! direction tags. Format:
//!
//! ```text
//! [1 byte: direction (0x00=input, 0x01=output)]
//! [8 bytes: timestamp (microseconds since epoch, LE u64)]
//! [4 bytes: length (LE u32)]
//! [{length} bytes: raw terminal data]
//! ```

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use capsem_foundation::unix::fs::{open_single_link_append_no_follow as open_append, open_single_link_file_no_follow};

/// Direction tag for PTY log entries.
pub const DIR_INPUT: u8 = 0x00;
pub const DIR_OUTPUT: u8 = 0x01;

/// Default max size before rotation (20 MB).
const DEFAULT_MAX_BYTES: u64 = 20 * 1024 * 1024;
// Include an overshooting final frame, while refusing corrupt or previously
// unbounded transcripts before allocating their claimed size.
const MAX_READ_BYTES: u64 = 2 * DEFAULT_MAX_BYTES;

/// Thread-safe PTY transcript writer with rotation support.
pub struct PtyLog {
    inner: Mutex<PtyLogInner>,
}

struct PtyLogInner {
    file: File,
    path: PathBuf,
    bytes_written: u64,
    max_bytes: u64,
}

impl PtyLog {
    /// Open (or create) a PTY log file at the given path.
    pub fn open(path: &Path) -> std::io::Result<Self> {
        Self::open_with_max(path, DEFAULT_MAX_BYTES)
    }

    /// Open with a custom max size before rotation.
    pub fn open_with_max(path: &Path, max_bytes: u64) -> std::io::Result<Self> {
        let file = open_append(path)?;
        let existing_size = file.metadata()?.len();
        Ok(Self {
            inner: Mutex::new(PtyLogInner {
                file,
                path: path.to_path_buf(),
                bytes_written: existing_size,
                max_bytes,
            }),
        })
    }

    /// Record terminal output (guest -> host).
    pub fn record_output(&self, data: &[u8]) {
        self.record(DIR_OUTPUT, data);
    }

    /// Record terminal input (host -> guest).
    pub fn record_input(&self, data: &[u8]) {
        self.record(DIR_INPUT, data);
    }

    fn record(&self, direction: u8, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        let timestamp_us = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_micros() as u64;
        let len = data.len() as u32;

        // Frame: direction(1) + timestamp(8) + length(4) + data(len)
        let frame_size = 1 + 8 + 4 + data.len();
        let mut frame = Vec::with_capacity(frame_size);
        frame.push(direction);
        frame.extend_from_slice(&timestamp_us.to_le_bytes());
        frame.extend_from_slice(&len.to_le_bytes());
        frame.extend_from_slice(data);

        let mut inner = match self.inner.lock() {
            Ok(g) => g,
            Err(_) => return, // poisoned mutex, skip
        };
        let _ = inner.file.write_all(&frame);
        inner.bytes_written += frame_size as u64;

        // Rotate if needed
        if inner.bytes_written >= inner.max_bytes {
            self.rotate_locked(&mut inner);
        }
    }

    fn rotate_locked(&self, inner: &mut PtyLogInner) {
        let rotated = inner.path.with_extension("log.1");
        // Best-effort: rename current -> .1, open fresh
        if std::fs::rename(&inner.path, &rotated).is_ok() {
            if let Ok(new_file) = open_append(&inner.path) {
                inner.file = new_file;
                inner.bytes_written = 0;
            }
        }
    }

    /// Current byte count written to the active log file.
    #[cfg(test)]
    pub fn bytes_written(&self) -> u64 {
        self.inner.lock().map(|g| g.bytes_written).unwrap_or(0)
    }
}

/// Read and iterate over PTY log entries from a file.
/// Returns entries as (direction, timestamp_us, data).
pub fn read_pty_log(path: &Path) -> std::io::Result<Vec<(u8, u64, Vec<u8>)>> {
    use std::io::Read;
    let file = open_single_link_file_no_follow(path)?;
    if file.metadata()?.len() > MAX_READ_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "transcript exceeds read limit",
        ));
    }
    let mut all = Vec::new();
    file.take(MAX_READ_BYTES + 1).read_to_end(&mut all)?;
    if all.len() as u64 > MAX_READ_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "transcript exceeds read limit",
        ));
    }
    parse_pty_log(&all)
}

/// Parse PTY log bytes into entries.
pub fn parse_pty_log(data: &[u8]) -> std::io::Result<Vec<(u8, u64, Vec<u8>)>> {
    let mut entries = Vec::new();
    let mut pos = 0;
    while pos + 13 <= data.len() {
        let direction = data[pos];
        let timestamp_us = u64::from_le_bytes(data[pos + 1..pos + 9].try_into().unwrap());
        let len = u32::from_le_bytes(data[pos + 9..pos + 13].try_into().unwrap()) as usize;
        if pos + 13 + len > data.len() {
            break; // truncated entry
        }
        let payload = data[pos + 13..pos + 13 + len].to_vec();
        entries.push((direction, timestamp_us, payload));
        pos += 13 + len;
    }
    Ok(entries)
}

/// Extract only output-direction data from a PTY log file: what the terminal
/// showed, without the keystrokes that produced it. This is what the
/// service's transcript route returns.
pub fn read_output_bytes(path: &Path) -> std::io::Result<Vec<u8>> {
    let entries = read_pty_log(path)?;
    let mut output = Vec::new();
    for (dir, _, data) in entries {
        if dir == DIR_OUTPUT {
            output.extend_from_slice(&data);
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests;
