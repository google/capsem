pub(crate) fn clone_fd(connection: &capsem_core::VsockConnection, operation: &'static str) -> Option<std::fs::File> {
    match connection.try_clone_file() {
        Ok(file) => Some(file),
        Err(error) => {
            tracing::warn!(
                operation,
                port = connection.port,
                errno = error.raw_os_error(),
                error = %error,
                "vsock descriptor duplication failed"
            );
            None
        }
    }
}

#[cfg(test)]
mod tests;
