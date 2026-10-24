//! Bounded response capture without cancelling an admitted package transaction.

use super::{api, UpdateCommandKind};
use crate::AppError;
use axum::http::StatusCode;
use std::io;
use std::process::{ExitStatus, Output, Stdio};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::{Child, Command};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Copy)]
struct Limits {
    timeout: Duration,
    output: usize,
}

impl Limits {
    fn configured() -> Self {
        Self {
            timeout: Duration::from_secs(capsem_proto::UPDATE_CHECK_TIMEOUT_SECONDS as u64),
            output: capsem_proto::UPDATE_OUTPUT_BYTES_PER_STREAM,
        }
    }
}

pub(crate) struct Completion {
    pub succeeded: bool,
    pub response: Result<api::UpdateActionResponse, AppError>,
}

#[derive(Debug)]
struct Captured {
    status: ExitStatus,
    output: io::Result<Output>,
}

fn command_error(error: io::Error) -> AppError {
    AppError::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("update command failed: {error}"),
    )
}

pub(crate) async fn execute(
    plan: api::UpdateCommandPlan,
    kind: UpdateCommandKind,
    cancelled: CancellationToken,
) -> Result<Completion, AppError> {
    let captured = run(&plan, kind, cancelled, Limits::configured())
        .await
        .map_err(command_error)?;
    let succeeded = captured.status.success();
    let response = captured
        .output
        .map(|output| api::UpdateActionResponse {
            status: if output.status.success() {
                api::UpdateActionStatus::Succeeded
            } else {
                api::UpdateActionStatus::Failed
            },
            command: plan,
            exit_code: output.status.code(),
            stdout: Some(String::from_utf8_lossy(&output.stdout).into_owned()),
            stderr: Some(String::from_utf8_lossy(&output.stderr).into_owned()),
        })
        .map_err(command_error);
    Ok(Completion { succeeded, response })
}

async fn read_pipe(mut pipe: impl AsyncRead + Unpin, limit: usize, drain: bool) -> io::Result<(Vec<u8>, bool)> {
    let mut retained = Vec::new();
    let mut exceeded = false;
    let mut buffer = [0; 4096];
    loop {
        let count = pipe.read(&mut buffer).await?;
        if count == 0 {
            return Ok((retained, exceeded));
        }
        let available = limit.saturating_sub(retained.len());
        retained.extend_from_slice(&buffer[..count.min(available)]);
        if count > available {
            exceeded = true;
            if !drain {
                return Err(io::Error::other("update output exceeds configured byte limit"));
            }
        }
    }
}

async fn capture(child: &mut Child, limit: usize, drain: bool) -> io::Result<Captured> {
    let stdout = child.stdout.take().expect("stdout was configured as a pipe");
    let stderr = child.stderr.take().expect("stderr was configured as a pipe");
    let (stdout, stderr, status) = if drain {
        // An error in one lane must not drop the other pipe or end ownership
        // of a running installer. Await the actual child before reporting it.
        let (out, err, status) = tokio::join!(
            read_pipe(stdout, limit, true),
            read_pipe(stderr, limit, true),
            child.wait()
        );
        (out, err, status?)
    } else {
        let (out, err, status) = tokio::try_join!(
            read_pipe(stdout, limit, false),
            read_pipe(stderr, limit, false),
            child.wait()
        )?;
        (Ok(out), Ok(err), status)
    };
    // Keep actual child success separate from response-capture failure: a
    // successful installer still requires runtime reconciliation on overflow.
    let output = (|| {
        let stdout = stdout?;
        let stderr = stderr?;
        if stdout.1 || stderr.1 {
            return Err(io::Error::other("update output exceeds configured byte limit"));
        }
        Ok(Output {
            status,
            stdout: stdout.0,
            stderr: stderr.0,
        })
    })();
    Ok(Captured { status, output })
}

async fn run(
    plan: &api::UpdateCommandPlan,
    kind: UpdateCommandKind,
    cancelled: CancellationToken,
    limits: Limits,
) -> io::Result<Captured> {
    let check = matches!(kind, UpdateCommandKind::Check);
    if check && cancelled.is_cancelled() {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "update check cancelled before startup",
        ));
    }
    let mut child = Command::new(&plan.program)
        .args(&plan.args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(check)
        .spawn()?;
    if !check {
        return capture(&mut child, limits.output, true).await;
    }
    let result = {
        let captured = capture(&mut child, limits.output, false);
        tokio::select! {
            result = tokio::time::timeout(limits.timeout, captured) => result.unwrap_or_else(|_| Err(io::Error::new(io::ErrorKind::TimedOut, "update check deadline exceeded"))),
            () = cancelled.cancelled() => Err(io::Error::new(io::ErrorKind::Interrupted, "update check cancelled")),
        }
    };
    if result.is_err() {
        // kill() waits for this exact child. Keep the transaction guard in
        // the service owner until termination and positive reap complete.
        if child.try_wait()?.is_none() {
            child.kill().await?;
        }
    }
    result
}

#[cfg(test)]
mod tests;
