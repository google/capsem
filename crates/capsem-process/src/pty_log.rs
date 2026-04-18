//! Append-only PTY transcript recording.
//!
//! Records all terminal I/O (both input and output) with timestamps and
//! direction tags. Format:
//!
//! ```text
//! [1 byte: direction (0x00=input, 0x01=output)]
//! [8 bytes: timestamp (microseconds since epoch, LE u64)]
//! [4 bytes: length (LE u32)]
//! [{length} bytes: raw terminal data]
//! ```

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

/// Direction tag for PTY log entries.
const DIR_INPUT: u8 = 0x00;
const DIR_OUTPUT: u8 = 0x01;

/// Default max size before rotation (20 MB).
const DEFAULT_MAX_BYTES: u64 = 20 * 1024 * 1024;

/// Thread-safe PTY transcript writer with rotation support.
pub(crate) struct PtyLog {
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
    pub(crate) fn open(path: &Path) -> std::io::Result<Self> {
        Self::open_with_max(path, DEFAULT_MAX_BYTES)
    }

    /// Open with a custom max size before rotation.
    pub(crate) fn open_with_max(path: &Path, max_bytes: u64) -> std::io::Result<Self> {
        let existing_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        let file = open_append(path)?;
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
    pub(crate) fn record_output(&self, data: &[u8]) {
        self.record(DIR_OUTPUT, data);
    }

    /// Record terminal input (host -> guest).
    pub(crate) fn record_input(&self, data: &[u8]) {
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
    pub(crate) fn bytes_written(&self) -> u64 {
        self.inner.lock().map(|g| g.bytes_written).unwrap_or(0)
    }
}

#[cfg(unix)]
fn open_append(path: &Path) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn open_append(path: &Path) -> std::io::Result<File> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
}

/// Read and iterate over PTY log entries from a file.
/// Returns entries as (direction, timestamp_us, data).
#[allow(dead_code)] // used by pty_replay and tests
pub(crate) fn read_pty_log(path: &Path) -> std::io::Result<Vec<(u8, u64, Vec<u8>)>> {
    use std::io::Read;
    let mut file = File::open(path)?;
    let mut all = Vec::new();
    file.read_to_end(&mut all)?;
    parse_pty_log(&all)
}

/// Parse PTY log bytes into entries.
#[allow(dead_code)] // used by read_pty_log and tests
fn parse_pty_log(data: &[u8]) -> std::io::Result<Vec<(u8, u64, Vec<u8>)>> {
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

/// Extract only output-direction data from a PTY log file.
/// Returns concatenated output bytes suitable for VTE replay.
#[allow(dead_code)] // used by pty_replay
pub(crate) fn read_output_bytes(path: &Path) -> std::io::Result<Vec<u8>> {
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
mod tests {
    use super::*;

    #[test]
    fn record_and_read_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pty.log");

        let log = PtyLog::open(&path).unwrap();
        log.record_output(b"hello from guest\r\n");
        log.record_input(b"ls -la\n");
        log.record_output(b"total 42\r\n");
        drop(log);

        let entries = read_pty_log(&path).unwrap();
        assert_eq!(entries.len(), 3);

        assert_eq!(entries[0].0, DIR_OUTPUT);
        assert_eq!(entries[0].2, b"hello from guest\r\n");

        assert_eq!(entries[1].0, DIR_INPUT);
        assert_eq!(entries[1].2, b"ls -la\n");

        assert_eq!(entries[2].0, DIR_OUTPUT);
        assert_eq!(entries[2].2, b"total 42\r\n");
    }

    #[test]
    fn read_output_bytes_filters_input() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pty.log");

        let log = PtyLog::open(&path).unwrap();
        log.record_output(b"output1");
        log.record_input(b"input1");
        log.record_output(b"output2");
        drop(log);

        let output = read_output_bytes(&path).unwrap();
        assert_eq!(output, b"output1output2");
    }

    #[test]
    fn rotation_at_max_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pty.log");

        // Set tiny max to trigger rotation
        let log = PtyLog::open_with_max(&path, 100).unwrap();
        log.record_output(&[0x41; 80]); // 80 + 13 header = 93 bytes
        assert!(log.bytes_written() > 0);

        log.record_output(&[0x42; 80]); // triggers rotation
        drop(log);

        // Rotated file should exist
        let rotated = dir.path().join("pty.log.1");
        assert!(rotated.exists(), "rotated file should exist");
        // New file should have only the post-rotation data
        assert!(path.exists(), "current file should exist");
    }

    #[test]
    fn empty_data_not_recorded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pty.log");

        let log = PtyLog::open(&path).unwrap();
        log.record_output(b"");
        log.record_input(b"");
        drop(log);

        let entries = read_pty_log(&path).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn timestamps_are_monotonic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pty.log");

        let log = PtyLog::open(&path).unwrap();
        for _ in 0..10 {
            log.record_output(b"x");
        }
        drop(log);

        let entries = read_pty_log(&path).unwrap();
        assert_eq!(entries.len(), 10);
        for i in 1..entries.len() {
            assert!(entries[i].1 >= entries[i - 1].1, "timestamps must be monotonic");
        }
    }

    #[test]
    fn binary_data_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pty.log");

        let binary: Vec<u8> = (0..=255).collect();
        let log = PtyLog::open(&path).unwrap();
        log.record_output(&binary);
        drop(log);

        let entries = read_pty_log(&path).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].2, binary);
    }
}
