use super::*;

#[test]
fn a_successful_empty_directory_is_enumerable() {
    let directory = tempfile::tempdir().unwrap();
    enumerate(directory.path()).unwrap();
}

#[test]
fn a_real_nonempty_directory_is_enumerable() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("fixture"), b"synthetic").unwrap();
    enumerate(directory.path()).unwrap();
}

#[test]
fn a_failed_read_directory_remains_an_error() {
    let directory = tempfile::tempdir().unwrap();
    let missing = directory.path().join("missing");
    assert_eq!(enumerate(&missing).unwrap_err().kind(), io::ErrorKind::NotFound);
}

#[test]
fn an_iteration_error_is_not_a_warmed_directory() {
    let entries = std::iter::once(Err::<(), _>(io::Error::from_raw_os_error(5)));
    assert_eq!(first_entry(entries).unwrap_err().raw_os_error(), Some(5));
}
