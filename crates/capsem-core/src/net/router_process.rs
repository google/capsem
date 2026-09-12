//! Starting a confined `capsem-router` child: the VM owner's pair relay and
//! the service's per-network switch are the same binary, spawned the same
//! way, and neither may forward a byte before it has confirmed its sandbox.
use anyhow::{Context, Result};
use capsem_foundation::unix::router_channel::Sender;
use capsem_router::{send_grant, Event, Grant};
use std::process::Stdio;
use std::time::Duration;
use tokio::net::UnixStream;

pub struct Confined {
    pub child: tokio::process::Child,
    pub pid: u32,
    pub sender: Sender,
    pub events: UnixStream,
}

/// Spawn the router beside this executable with `args`, and wait for it to
/// report a confined, ready process.
pub async fn spawn(args: &[String]) -> Result<Confined> {
    let binary = std::env::current_exe()?.with_file_name("capsem-router");
    let (parent, child_socket) = std::os::unix::net::UnixStream::pair()?;
    let mut child = tokio::process::Command::new(binary)
        .args(["--parent-pid", &std::process::id().to_string()])
        .args(args)
        .env_clear()
        .current_dir("/")
        .stdin(Stdio::from(std::os::fd::OwnedFd::from(child_socket)))
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .context("start confined router")?;
    let pid = child.id().context("router exited during startup")?;
    parent.set_nonblocking(true)?;
    let sender = Sender::new(parent.try_clone()?)?;
    let mut events = UnixStream::from_std(parent)?;
    let started = tokio::time::timeout(Duration::from_secs(5), async {
        send_grant(&sender, Grant::Hello).await?;
        match Event::read(&mut events).await.context("read router startup response")? {
            Event::Ready => Ok(()),
            Event::ConfinementFailed => anyhow::bail!("router could not install its sandbox"),
            _ => anyhow::bail!("router did not confirm confinement"),
        }
    })
    .await
    .context("router startup timed out");
    if let Err(error) = started.and_then(|started| started) {
        let _ = child.kill().await;
        return Err(error);
    }
    Ok(Confined {
        child,
        pid,
        sender,
        events,
    })
}
