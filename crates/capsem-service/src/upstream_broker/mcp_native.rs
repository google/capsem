//! The service retains every native MCP child until generation retirement.
use super::{mcp_http, mcp_spawn, BrokerPolicy, WorkerGrant};
use crate::instance::WorkerAuthority;
use anyhow::{Context, Result};
use capsem_mcp_transport::native_worker::{grant_worker, STARTUP_TIMEOUT};
use capsem_proto::mcp_worker::{McpHttpRole, McpWorkerBootstrap};
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use tokio::sync::watch;

pub(super) struct Prepared {
    pub(super) builtin: mcp_spawn::Factory,
    pub(super) aggregator: PathBuf,
    pub(super) stderr: std::fs::File,
    pub(super) lock: PathBuf,
    trace_env: Vec<(String, String)>,
}

pub(super) struct Supervisor {
    child: tokio::process::Child,
    http: tokio::task::JoinHandle<()>,
    spawn: tokio::task::JoinHandle<Result<(), String>>,
    scope: WorkerAuthority,
    generation: WorkerGrant,
    policy: watch::Receiver<Arc<BrokerPolicy>>,
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        self.scope.revoke();
        self.http.abort();
        // The spawn owner observes scope cancellation and positively reaps its
        // children. Aborting it would discard that completion evidence.
    }
}

impl Prepared {
    pub(super) fn prepare(session: &Path, capacity: Arc<tokio::sync::Semaphore>) -> Result<Self> {
        let builtin = mcp_spawn::Factory::prepare(session, capacity)?;
        let trace_env = builtin.trace_env.clone();
        Ok(Self {
            builtin,
            aggregator: crate::service_runtime::find_sibling_binary("capsem-mcp-aggregator"),
            stderr: capsem_foundation::unix::fs::open_private_append_no_follow(
                &session.join("mcp-aggregator.stderr.log"),
            )?,
            lock: session.join("mcp-aggregator.lock"),
            trace_env,
        })
    }
    fn launch(
        self,
        policy: watch::Receiver<Arc<BrokerPolicy>>,
        generation: WorkerGrant,
    ) -> Result<(Supervisor, UnixStream, UnixStream, UnixStream)> {
        let (control, child_control) = UnixStream::pair()?;
        let (http, child_http) = UnixStream::pair()?;
        let (spawn, child_spawn) = UnixStream::pair()?;
        let scope = WorkerAuthority::default();
        let mut command = tokio::process::Command::new(self.aggregator);
        command
            .env_clear()
            .envs(self.trace_env)
            .args(["--parent-pid", &std::process::id().to_string(), "--lock-path"])
            .arg(self.lock)
            .stdin(Stdio::from(OwnedFd::from(child_control)))
            .stdout(Stdio::null())
            .stderr(Stdio::from(self.stderr))
            .kill_on_drop(true);
        let child = command.spawn().context("spawn confined native MCP aggregator")?;
        drop(command);
        let http = tokio::spawn(mcp_http::serve(
            http,
            McpHttpRole::Aggregator,
            policy.clone(),
            scope.grant(),
        ));
        let spawn = tokio::spawn(mcp_spawn::serve(spawn, self.builtin, policy.clone(), scope.grant()));
        Ok((
            Supervisor {
                child,
                http,
                spawn,
                scope,
                generation,
                policy,
            },
            control,
            child_http,
            child_spawn,
        ))
    }

    pub(super) async fn open(
        self,
        policy: watch::Receiver<Arc<BrokerPolicy>>,
        generation: WorkerGrant,
    ) -> (Result<UnixStream>, Option<Supervisor>) {
        let (supervisor, control, http, spawn) = match self.launch(policy, generation) {
            Ok(prepared) => prepared,
            Err(error) => return (Err(error), None),
        };
        let bootstrap = McpWorkerBootstrap {
            role: McpHttpRole::Aggregator,
            generation: capsem_crypto::random::uuid_v4().into_bytes(),
        };
        let descriptors = [http.as_fd(), spawn.as_fd()];
        let ready = tokio::select! {
            biased;
            () = supervisor.generation.revoked() => Err(anyhow::anyhow!("MCP generation retired during startup")),
            result = grant_worker(&control, bootstrap, &descriptors) => result,
        };
        (ready.map(|()| control), Some(supervisor))
    }
}

impl Supervisor {
    pub(super) async fn run(mut self, stop_now: bool) -> Result<(), String> {
        if !stop_now {
            let mut policy = self.policy.clone();
            let closed = async move { while policy.changed().await.is_ok() {} };
            tokio::select! {
                biased;
                () = self.generation.revoked() => {},
                () = closed => {},
                result = self.child.wait() => {
                    if let Err(error) = result { tracing::warn!(%error, "native MCP aggregator wait failed"); }
                }
            }
        }
        self.scope.revoke();
        let mut failures = Vec::new();
        if let Err(error) = self.child.start_kill() {
            failures.push(format!("stop MCP aggregator: {error}"));
        }
        match tokio::time::timeout(STARTUP_TIMEOUT, self.child.wait()).await {
            Ok(Ok(_)) => {}
            result => failures.push(format!("MCP aggregator exit unconfirmed: {result:?}")),
        }
        self.http.abort();
        let _ = (&mut self.http).await;
        match (&mut self.spawn).await {
            Ok(Ok(())) => {}
            result => failures.push(format!("MCP builtin exit unconfirmed: {result:?}")),
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(failures.join("; "))
        }
    }
}

#[cfg(test)]
mod tests;
