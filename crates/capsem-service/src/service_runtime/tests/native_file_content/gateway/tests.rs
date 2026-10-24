use super::*;

#[test]
fn original_read_diagnostics_never_render_held_bytes_or_raw_errors() {
    for (data, error, expected) in [
        (Some(BYTES.to_vec()), None, "read_completed"),
        (
            None,
            Some(format!("FileRead rejected: {}", String::from_utf8_lossy(BYTES))),
            "guest_path_refused",
        ),
        (
            None,
            Some(format!("file export refused: {}", String::from_utf8_lossy(BYTES))),
            "security_boundary_refused",
        ),
    ] {
        let summary = read_summary(&Ok(ProcessToService::ReadFileResult { id: 1, data, error }));
        assert!(summary.starts_with(expected));
        assert!(!summary.contains(std::str::from_utf8(BYTES).unwrap()));
    }
}

#[test]
fn transport_refusal_diagnostic_does_not_echo_its_cause() {
    let summary = read_summary(&Err(crate::vm_files::ipc_command::IpcCommandError::Failed(
        String::from_utf8_lossy(BYTES).into_owned(),
    )));
    assert_eq!(summary, "ipc_failure");
}
