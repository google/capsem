//! One writer per ledger, enforced rather than assumed.
//!
//! A ledger is `session.db` and the `session.bodies` archive beside it, and
//! only the archive's own writer knows where the file ends: it reads the end
//! once at open and places every segment it writes from there. A second writer
//! on the same ledger appends too, so the first segment it writes moves the real
//! end under the first writer, and every index row the first writes afterwards
//! names bytes that are not its body. SQLite would have serialised the two
//! writers without complaint; the archive cannot, and nothing noticed until a
//! measurement found the builtin MCP server holding the archive open for
//! writing beside capsem-process in every sample.
//!
//! So a writer takes an exclusive lock on a sidecar file before it touches the
//! ledger, and its writer thread holds it until the thread ends -- after the
//! last block is closed and the last row committed. A second writer, in this
//! process or any other, fails at open and names the ledger, instead of
//! corrupting it later and silently.
//!
//! The lock is on a sidecar rather than on `session.bodies` because retention
//! replaces the archive with `rename(2)`: a lock on the old inode would stop
//! guarding the new file the moment compaction finished. The kernel drops a
//! `flock` when its holder dies, so a crashed writer leaves nothing to clean up.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use capsem_foundation::unix::lock::{self, FileLock, LockAttempt, LockMode};

/// How long an opening writer waits for the previous one to finish closing:
/// long enough for a process that is exiting to seal its last block, short
/// enough that a genuine second writer is refused promptly.
const WAIT_FOR_PREVIOUS_WRITER: Duration = Duration::from_secs(1);
const RETRY_EVERY: Duration = Duration::from_millis(20);

/// The sidecar a ledger's writer holds, beside the database.
pub(crate) fn writer_lock_path(db_path: &Path) -> PathBuf {
    let mut name = OsString::from(db_path.as_os_str());
    name.push("-writer.lock");
    PathBuf::from(name)
}

/// Take the ledger's writer lock, waiting briefly for a writer that is
/// closing, and refuse -- naming the ledger -- if another writer holds it.
pub(crate) fn acquire(db_path: &Path) -> rusqlite::Result<FileLock> {
    let lock_path = writer_lock_path(db_path);
    let deadline = Instant::now() + WAIT_FOR_PREVIOUS_WRITER;
    loop {
        match lock::try_acquire(&lock_path, LockMode::Exclusive) {
            Ok(LockAttempt::Acquired(held)) => return Ok(held),
            Ok(LockAttempt::Contended) if Instant::now() < deadline => std::thread::sleep(RETRY_EVERY),
            Ok(LockAttempt::Contended) => {
                return Err(refused(
                    db_path,
                    &format!("another writer holds {}", lock_path.display()),
                ))
            }
            Err(error) => return Err(refused(db_path, &format!("{}: {error}", lock_path.display()))),
        }
    }
}

fn refused(db_path: &Path, why: &str) -> rusqlite::Error {
    rusqlite::Error::InvalidParameterName(format!(
        "ledger {} already has a writer ({why}); a ledger has exactly one writer, because its \
         body archive cannot survive two appenders",
        db_path.display()
    ))
}

#[cfg(test)]
mod tests;
