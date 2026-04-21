use std::os::unix::io::RawFd;

pub(crate) fn clone_fd(fd: RawFd) -> std::io::Result<std::fs::File> {
    use std::os::unix::io::FromRawFd;
    if fd == -1 {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid file descriptor -1"));
    }
    let file = std::mem::ManuallyDrop::new(unsafe { std::fs::File::from_raw_fd(fd) });
    file.try_clone()
}

pub(crate) fn query_max_fs_event_id(db: &capsem_logger::DbWriter) -> i64 {
    db.reader().ok()
        .and_then(|r| r.query_raw("SELECT COALESCE(MAX(id),0) FROM fs_events").ok())
        .and_then(|json| {
            let parsed: serde_json::Value = serde_json::from_str(&json).ok()?;
            parsed["rows"].get(0)?.get(0)?.as_i64()
        })
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clone_fd_valid_file() {
        use std::io::Write;
        use std::os::unix::io::AsRawFd;
        // Use a pipe as a valid FD source
        let (read_fd, write_fd) = nix::unistd::pipe().unwrap();
        let raw_write = write_fd.as_raw_fd();
        let _raw_read = read_fd.as_raw_fd();
        let mut cloned = clone_fd(raw_write).unwrap();
        cloned.write_all(b"test").unwrap();
        drop(read_fd);
        drop(write_fd);
    }

    #[test]
    fn clone_fd_invalid_fd_fails() {
        // -1 is universally an invalid file descriptor in POSIX.
        // This avoids multithreaded race conditions where a closed FD
        // is instantly reused by another test.
        let result = clone_fd(-1);
        assert!(result.is_err());
    }

    #[test]
    fn query_max_fs_event_id_on_empty_db_is_zero() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("empty.db");
        let writer = capsem_logger::DbWriter::open(&db_path, 16).unwrap();
        assert_eq!(query_max_fs_event_id(&writer), 0);
    }

    #[test]
    fn query_max_fs_event_id_reflects_highest_row() {
        use capsem_logger::writer::WriteOp;
        use capsem_logger::events::{FileEvent, FileAction};

        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("events.db");
        let writer = capsem_logger::DbWriter::open(&db_path, 64).unwrap();

        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        rt.block_on(async {
            for i in 0..3 {
                writer.write(WriteOp::FileEvent(FileEvent {
                    timestamp: std::time::SystemTime::now(),
                    action: FileAction::Created,
                    path: format!("/tmp/f{i}"),
                    size: Some(1),
                })).await;
            }
        });

        // Drop the writer so the batch is flushed and visible to the reader.
        drop(writer);

        // Reopen to query the final max id.
        let reader_writer = capsem_logger::DbWriter::open(&db_path, 16).unwrap();
        assert_eq!(query_max_fs_event_id(&reader_writer), 3);
    }
}
