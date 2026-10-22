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

use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;
use std::time::SystemTime;

use capsem_foundation::unix::contained::{ContainedDir, ContainedOpenOptions};
use capsem_foundation::unix::fs::open_single_link_file_no_follow;

mod recorder;
pub use recorder::PtyRecorder;

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
    directory: ContainedDir,
    name: OsString,
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
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let name = path
            .file_name()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "transcript needs a file name"))?;
        Self::open_with_max_in(ContainedDir::open_root(parent)?, name, max_bytes)
    }

    /// Keep the session-log directory open throughout recording and rotation.
    pub fn open_in(directory: ContainedDir, name: &OsStr) -> std::io::Result<Self> {
        Self::open_with_max_in(directory, name, DEFAULT_MAX_BYTES)
    }

    pub fn open_with_max_in(directory: ContainedDir, name: &OsStr, max_bytes: u64) -> std::io::Result<Self> {
        let file = directory.open_single_link_file(name, ContainedOpenOptions::append_create(0o600))?;
        let existing_size = file.metadata()?.len();
        Ok(Self {
            inner: Mutex::new(PtyLogInner {
                file,
                directory,
                name: name.to_owned(),
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
        let _ = self.write_frame(&record_frame(direction, data));
    }

    fn write_frame(&self, frame: &[u8]) -> std::io::Result<()> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| std::io::Error::other("PTY log lock poisoned"))?;
        inner.file.write_all(frame)?;
        inner.bytes_written += frame.len() as u64;
        if inner.bytes_written >= inner.max_bytes {
            Self::rotate_locked(&mut inner)?;
        }
        drop(inner);
        Ok(())
    }

    fn rotate_locked(inner: &mut PtyLogInner) -> std::io::Result<()> {
        let rotated = Path::new(&inner.name).with_extension("log.1").into_os_string();
        inner.directory.rename_to(&inner.name, &inner.directory, &rotated)?;
        inner.file = inner
            .directory
            .open_single_link_file(&inner.name, ContainedOpenOptions::append_create(0o600))?;
        inner.bytes_written = 0;
        Ok(())
    }

    /// Current byte count written to the active log file.
    #[cfg(test)]
    pub fn bytes_written(&self) -> u64 {
        self.inner.lock().map(|g| g.bytes_written).unwrap_or(0)
    }
}

fn timestamp_us() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64
}

fn record_frame(direction: u8, data: &[u8]) -> Vec<u8> {
    let timestamp_us = timestamp_us();
    let len = data.len() as u32;

    // Frame: direction(1) + timestamp(8) + length(4) + data(len)
    let frame_size = 1 + 8 + 4 + data.len();
    let mut frame = Vec::with_capacity(frame_size);
    frame.push(direction);
    frame.extend_from_slice(&timestamp_us.to_le_bytes());
    frame.extend_from_slice(&len.to_le_bytes());
    frame.extend_from_slice(data);

    frame
}

/// Read and iterate over PTY log entries from a file.
/// Returns entries as (direction, timestamp_us, data).
pub fn read_pty_log(path: &Path) -> std::io::Result<Vec<(u8, u64, Vec<u8>)>> {
    read_pty_log_file(open_single_link_file_no_follow(path)?)
}

/// Decode an already opened transcript without resolving its pathname again.
pub fn read_pty_log_file(file: File) -> std::io::Result<Vec<(u8, u64, Vec<u8>)>> {
    use std::io::Read;
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
    read_output_file(open_single_link_file_no_follow(path)?)
}

pub fn read_output_file(file: File) -> std::io::Result<Vec<u8>> {
    let entries = read_pty_log_file(file)?;
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
