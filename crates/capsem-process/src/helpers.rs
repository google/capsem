use std::os::unix::io::RawFd;

pub(crate) fn clone_fd(fd: RawFd) -> std::io::Result<std::fs::File> {
    use std::os::unix::io::FromRawFd;
    if fd == -1 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "invalid file descriptor -1",
        ));
    }
    let file = std::mem::ManuallyDrop::new(unsafe { std::fs::File::from_raw_fd(fd) });
    file.try_clone()
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
}
