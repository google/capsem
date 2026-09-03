use super::*;

/// GET /vms/{id}/history/transcript -- the last `tail_lines` lines the
/// terminal showed, base64-encoded.
///
/// `pty.log` is capsem-process's framed record of both directions; the route
/// used to return those raw frames, headers and keystrokes included, and
/// ignored `tail_lines`. It now decodes the output entries through the
/// shared parser, takes the tail on the blocking pool, and reports the bytes
/// of the slice it returned.
pub(crate) async fn handle_history_transcript(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    Query(params): Query<api::TranscriptQuery>,
) -> Result<Json<api::TranscriptResponse>, AppError> {
    use base64::Engine;
    let session_dir = resolve_session_dir(&state, &id)?;
    let pty_log_path = session_dir.join("pty.log");

    if !pty_log_path.exists() {
        return Ok(Json(api::TranscriptResponse {
            content: String::new(),
            bytes: 0,
        }));
    }

    let lines = params.tail_lines;
    let output = tokio::task::spawn_blocking(move || {
        capsem_core::pty_log::read_output_bytes(&pty_log_path).map(|shown| tail_lines(&shown, lines).to_vec())
    })
    .await
    .map_err(|e| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("transcript read task failed: {e}"),
        )
    })?
    .map_err(|e| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to read pty.log: {e}"),
        )
    })?;

    let encoded = base64::engine::general_purpose::STANDARD.encode(&output);
    Ok(Json(api::TranscriptResponse {
        bytes: output.len(),
        content: encoded,
    }))
}

/// The last `lines` lines of `bytes`. A terminating newline ends the last
/// line rather than starting an empty one, and an unterminated final line
/// still counts as a line.
fn tail_lines(bytes: &[u8], lines: usize) -> &[u8] {
    if lines == 0 {
        return &[];
    }
    let scan_end = bytes.len() - usize::from(bytes.ends_with(b"\n"));
    let mut remaining = lines;
    let mut start = 0;
    for (index, byte) in bytes[..scan_end].iter().enumerate().rev() {
        if *byte == b'\n' {
            remaining -= 1;
            if remaining == 0 {
                start = index + 1;
                break;
            }
        }
    }
    &bytes[start..]
}
