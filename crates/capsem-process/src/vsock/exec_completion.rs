use super::{SecurityRulesHandle, EXEC_OUTPUT_DEPOSIT_TIMEOUT};
use crate::job_store::{JobResult, JobStore};
use std::sync::Arc;
use tracing::warn;

pub(super) async fn complete(
    id: u64,
    exit_code: i32,
    js: &Arc<JobStore>,
    db: &Arc<capsem_logger::DbWriter>,
    security_rules: &SecurityRulesHandle,
) {
    // A captured command retains the existing transport-loss bound. Streaming
    // may legitimately wait behind a slow consumer for longer; its connection or
    // VM lifecycle cancels the wait, never a timer that would silently lose logs.
    let pending = js
        .active_execs
        .lock()
        .unwrap()
        .get(&id)
        .map(|active| (active.deposited.clone(), active.stream.clone()));
    let Some((deposited, stream)) = pending else {
        return;
    };
    if let Some(sender) = &stream {
        tokio::select! {
            _ = deposited.notified() => {},
            _ = sender.closed() => {},
        }
    } else if tokio::time::timeout(EXEC_OUTPUT_DEPOSIT_TIMEOUT, deposited.notified())
        .await
        .is_err()
    {
        // The reader deposits at EOF even for a command that printed nothing,
        // so no deposit means the output was lost, not that it was empty.
        // Reporting an empty success here made lost output indistinguishable
        // from silence.
        warn!(
            exec_id = id,
            bound_ms = EXEC_OUTPUT_DEPOSIT_TIMEOUT.as_millis() as u64,
            "exec finished but its output never reached the host"
        );
        if let Some(active) = js.active_execs.lock().unwrap().get_mut(&id) {
            active.output_error.get_or_insert_with(|| {
                format!(
                    "exec output did not reach the host within {}s of the command finishing",
                    EXEC_OUTPUT_DEPOSIT_TIMEOUT.as_secs()
                )
            });
        }
    }
    let Some(active) = js.active_execs.lock().unwrap().remove(&id) else {
        return;
    };
    let event_id = active.event_id;
    let duration_ms = active.started_at.elapsed().as_millis() as u64;
    let stdout = active.captured;
    let stderr = active.captured_stderr;
    let stdout_bytes = active.total_bytes;
    let stderr_bytes = active.stderr_bytes;
    let streaming = stream.is_some();
    let truncated = !streaming && (stdout_bytes > stdout.len() as u64 || stderr_bytes > stderr.len() as u64);

    let complete = capsem_logger::ExecEventComplete {
        exec_id: id,
        exit_code,
        duration_ms,
        stdout_preview: Some(
            String::from_utf8_lossy(&stdout[..stdout.len().min(super::exec_output::EXEC_LEDGER_PREVIEW_BYTES)]).into(),
        ),
        stderr_preview: Some(
            String::from_utf8_lossy(&stderr[..stderr.len().min(super::exec_output::EXEC_LEDGER_PREVIEW_BYTES)]).into(),
        ),
        stdout_bytes,
        stderr_bytes,
        pid: None,
    };
    if let Some(event_id) = event_id {
        let rules = security_rules.read().unwrap().clone();
        capsem_core::security_engine::emit_process_complete_security_write_and_rules(db, &rules, event_id, complete)
            .await;
    } else {
        warn!(
            exec_id = id,
            "exec completion arrived without a primary security event id; updating exec row without rule ledger match"
        );
        capsem_core::security_engine::emit_process_complete_security_write_only(db, complete).await;
    }
    if let Some(tx) = js.jobs.lock().unwrap().remove(&id) {
        let result = if let Some(message) = active.output_error {
            JobResult::Error { message }
        } else if stream.as_ref().is_some_and(|sender| sender.is_closed()) {
            JobResult::Error {
                message: "exec stream consumer disconnected".into(),
            }
        } else {
            JobResult::Exec {
                stdout: if streaming { Vec::new() } else { stdout },
                stderr: if streaming { Vec::new() } else { stderr },
                exit_code,
                truncated,
            }
        };
        capsem_core::try_send!("job_result_exec", tx.send(result));
    }
}

#[cfg(test)]
mod tests;
