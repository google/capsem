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
    let stdout_bytes = active.total_bytes;
    let stderr_bytes = active.stderr_bytes;
    let streaming = stream.is_some();
    // A streamed exec already delivered its bytes; a buffered one returns the
    // leading slices one result frame carries. That copy is the only one, and
    // it is bounded by the result's cap, not by the ledger's.
    let (response_stdout, response_stderr) = active
        .response_cut
        .unwrap_or((active.captured.len(), active.captured_stderr.len()));
    let truncated = !streaming && (stdout_bytes > response_stdout as u64 || stderr_bytes > response_stderr as u64);
    let response = (!streaming).then(|| {
        (
            active.captured[..response_stdout].to_vec(),
            active.captured_stderr[..response_stderr].to_vec(),
        )
    });

    // The ledger takes the captured lanes themselves, moved, not copied: up to
    // the logger's body cap each, whatever the result or the stream carried.
    let complete = capsem_logger::ExecEventComplete {
        exec_id: id,
        exit_code,
        duration_ms,
        stdout: active.captured,
        stderr: active.captured_stderr,
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
            let (stdout, stderr) = response.unwrap_or_default();
            JobResult::Exec {
                stdout,
                stderr,
                exit_code,
                truncated,
            }
        };
        capsem_core::try_send!("job_result_exec", tx.send(result));
    }
}

#[cfg(test)]
mod tests;
