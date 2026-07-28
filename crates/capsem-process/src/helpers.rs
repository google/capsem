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
mod tests;
