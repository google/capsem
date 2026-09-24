//! Test-only fault injection at generation publication boundaries.
//!
//! Candidate sync and uncertain COMMIT outcomes do not fail on demand, so
//! deterministic tests inject them at the production ordering seams.
//!
//! A set of (archive, step) pairs, not one slot for the whole process. A
//! single slot made two tests that arm faults on different ledgers clobber
//! each other whenever they overlapped, and the loser passed: its retention
//! succeeded, which is exactly the outcome it was written to prove
//! impossible. The archive and step together are the key so concurrent
//! ledgers cannot consume each other's faults.
//!
//! Outside `cfg(test)` the hook compiles to a constant `false`, so the
//! retention path has one shape in both builds.

use std::path::Path;

/// Which step to fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum RetentionFault {
    /// Report the durable candidate sync as failed. SQLite must remain on G.
    CandidateSync,
    /// Fail inside the transaction that rewrites the index, before it
    /// commits. The archive must be left exactly as it was.
    IndexTransaction,
    /// Report COMMIT as uncertain while SQLite remains on G.
    CommitUnknownBefore,
    /// Report COMMIT as uncertain after SQLite has elected H.
    CommitUnknownAfter,
}

#[cfg(test)]
static RETENTION_FAULTS: std::sync::Mutex<Option<std::collections::HashSet<(std::path::PathBuf, RetentionFault)>>> =
    std::sync::Mutex::new(None);

/// Fail `fault` once, for the archive beside `db_path`.
#[cfg(test)]
pub(crate) fn fail_retention_for_path_for_tests(db_path: &Path, fault: RetentionFault) {
    let archive = super::bodies::archive_path_for_db(db_path);
    let archive = archive.canonicalize().unwrap_or(archive);
    RETENTION_FAULTS
        .lock()
        .unwrap()
        .get_or_insert_with(std::collections::HashSet::new)
        .insert((archive, fault));
}

#[cfg(test)]
pub(super) fn take_retention_failure_for_tests(archive_path: &Path, fault: RetentionFault) -> bool {
    let mut guard = RETENTION_FAULTS.lock().unwrap();
    let armed = guard
        .as_mut()
        .is_some_and(|faults| faults.remove(&(archive_path.to_path_buf(), fault)));
    drop(guard);
    armed
}

#[cfg(not(test))]
pub(super) fn take_retention_failure_for_tests(_archive_path: &Path, _fault: RetentionFault) -> bool {
    false
}
