//! Test-only fault injection for disk flushes.
//!
//! The handle tests need a flush to fail on demand, for one database or for
//! every one, so that barrier callers are proven not to report rows durable
//! that never reached disk. Outside `cfg(test)` the hook compiles to a
//! constant `false`, so the writer's flush path has one shape in both builds.

use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;

#[cfg(test)]
static FAIL_DISK_FLUSHES_FOR_TESTS: std::sync::Mutex<Option<(PathBuf, usize)>> = std::sync::Mutex::new(None);

#[cfg(test)]
pub(crate) fn fail_disk_flushes_for_tests(count: usize) {
    let mut guard = FAIL_DISK_FLUSHES_FOR_TESTS.lock().unwrap();
    if count == 0 {
        *guard = None;
    } else {
        *guard = Some((PathBuf::new(), count));
    }
}

#[cfg(test)]
pub(crate) fn fail_disk_flushes_for_path_for_tests(path: &Path, count: usize) {
    let mut guard = FAIL_DISK_FLUSHES_FOR_TESTS.lock().unwrap();
    if count == 0 {
        *guard = None;
    } else {
        *guard = Some((path.to_path_buf(), count));
    }
}

#[cfg(test)]
pub(super) fn take_disk_flush_failure_for_tests(db_path: Option<&Path>) -> bool {
    let mut guard = FAIL_DISK_FLUSHES_FOR_TESTS.lock().unwrap();
    let Some((configured_path, remaining)) = guard.as_mut() else {
        return false;
    };
    if *remaining == 0 {
        *guard = None;
        return false;
    }
    if !configured_path.as_os_str().is_empty() && db_path != Some(configured_path.as_path()) {
        return false;
    }
    *remaining -= 1;
    if *remaining == 0 {
        *guard = None;
    }
    true
}

#[cfg(not(test))]
pub(super) fn take_disk_flush_failure_for_tests(_db_path: Option<&Path>) -> bool {
    false
}
