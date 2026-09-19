//! Test-only fault injection for the two steps retention cannot retry.
//!
//! Retention's whole shape is an ordering: stage the replacement, commit the
//! index, then rename. What that ordering is *for* only shows up when one of
//! the last two steps fails, and neither fails on demand -- a full disk and a
//! read-only directory are not things a unit test can arrange around a writer
//! thread it does not own. So the two are injectable.
//!
//! A set of (archive, step) pairs, not one slot for the whole process. A
//! single slot made two tests that arm faults on different ledgers clobber
//! each other whenever they overlapped, and the loser passed: its retention
//! succeeded, which is exactly the outcome it was written to prove
//! impossible. The pair is the key rather than the path, because one test
//! arms two steps at once to reach the state where neither worked.
//!
//! Outside `cfg(test)` the hook compiles to a constant `false`, so the
//! retention path has one shape in both builds.

use std::path::Path;

/// Which step to fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum RetentionFault {
    /// Fail inside the transaction that rewrites the index, before it
    /// commits. The archive must be left exactly as it was.
    IndexTransaction,
    /// Fail the rename that puts the compacted archive in place, after the
    /// index has committed. The old offsets must come back.
    Rename,
    /// Fail putting the old offsets back after a failed rename. This is the
    /// one state nothing can repair, so the archive must stop accepting
    /// bodies rather than keep writing into a ledger it cannot vouch for.
    Restore,
}

#[cfg(test)]
static RETENTION_FAULTS: std::sync::Mutex<Option<std::collections::HashSet<(std::path::PathBuf, RetentionFault)>>> =
    std::sync::Mutex::new(None);

/// Fail `fault` once, for the archive beside `db_path`.
#[cfg(test)]
pub(crate) fn fail_retention_for_path_for_tests(db_path: &Path, fault: RetentionFault) {
    RETENTION_FAULTS
        .lock()
        .unwrap()
        .get_or_insert_with(std::collections::HashSet::new)
        .insert((super::bodies::archive_path_for_db(db_path), fault));
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
