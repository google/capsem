use std::collections::{BTreeMap, HashSet};
use std::io;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::process::Command;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use capsem_foundation::unix::worker_sandbox::{Policy, Role};
use capsem_foundation::unix::{fd, router_channel};
use capsem_proto::proxy_control::{
    decode_proxy_control_request, encode_proxy_control_event, ProxyCapability, ProxyControlEvent,
    ProxyControlRejection, ProxyControlRequest, ProxyGeneration, PROXY_CONTROL_FRAME_SIZE, PROXY_CONTROL_MAX_FDS,
};
use clap::Parser;

const CONTROL_QUEUE_CAPACITY: usize = 16;
const GRANT_LIMIT: usize = 70;
const TRAFFIC_GRANT_LIMIT: usize = 64;
const CONTROL_SEND_TIMEOUT: Duration = Duration::from_secs(5);

type ControlSender = router_channel::DescriptorSender<PROXY_CONTROL_FRAME_SIZE, PROXY_CONTROL_MAX_FDS>;
type ControlReceiver = router_channel::DescriptorReceiver<PROXY_CONTROL_FRAME_SIZE, PROXY_CONTROL_MAX_FDS>;
type ControlFrame = router_channel::DescriptorFrame<PROXY_CONTROL_FRAME_SIZE>;

#[derive(Parser)]
#[command(version, about = "Confined per-session Capsem proxy worker")]
struct Args {
    #[arg(long)]
    parent_pid: u32,
    /// Sixteen-byte worker generation as 32 lowercase hexadecimal digits.
    #[arg(long, value_parser = parse_generation)]
    generation: ProxyGeneration,
}

fn main() -> Result<()> {
    // SAFETY: process entry, before telemetry, runtimes or capability
    // channels can observe environment state or own unrelated descriptors.
    unsafe {
        capsem_foundation::unix::process::clear_inherited_environment();
        fd::close_inherited_descriptors()?;
    }
    let _telemetry = capsem_foundation::telemetry::init(capsem_foundation::telemetry::TelemetryConfig {
        service: "capsem-proxy",
        sink: capsem_foundation::telemetry::LogSink::Stderr,
        default_filter: "capsem_proxy=info,capsem_core=info,capsem_foundation=warn",
    })?;
    let args = Args::parse();
    capsem_guard::watch_parent_or_exit(Some(args.parent_pid))?;
    let stdin = io::stdin();
    let control = UnixStream::from(fd::duplicate(stdin.as_fd())?);
    let denied_file = std::env::current_exe().context("resolve proxy executable before confinement")?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    capsem_foundation::unix::worker_sandbox::confine(&Policy::new(Role::Proxy))
        .context("confine proxy worker before readiness")?;
    attest_confinement(&denied_file, args.parent_pid)?;
    runtime.block_on(run_control(control, args.generation))
}

fn attest_confinement(denied_file: &std::path::Path, parent_pid: u32) -> Result<()> {
    if std::env::vars_os().next().is_some() {
        bail!("proxy worker retained inherited environment state");
    }
    require_denied(std::fs::read(denied_file), "read an ambient file")?;
    require_denied(UnixStream::pair(), "create a Unix socket")?;
    require_denied(
        std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)),
        "create a TCP listener",
    )?;
    require_denied(
        std::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, 9)),
        "dial a TCP address",
    )?;
    require_denied(Command::new(denied_file).arg("--version").status(), "execute a process")?;
    let parent = capsem_foundation::unix::process::ProcessId::try_from(parent_pid)?;
    if capsem_foundation::unix::process::probe_signal_authority(parent)?
        != capsem_foundation::unix::process::SignalProbe::Denied
    {
        bail!("proxy confinement allowed it to signal its parent");
    }
    Ok(())
}

fn require_denied<T>(result: io::Result<T>, operation: &str) -> Result<()> {
    if result.is_ok() {
        bail!("proxy confinement allowed it to {operation}");
    }
    Ok(())
}

async fn run_control(control: UnixStream, generation: ProxyGeneration) -> Result<()> {
    let shutdown = control.try_clone()?;
    let sender = ControlSender::new(control.try_clone()?)?;
    let receiver = ControlReceiver::new(control)?;
    let (frames_sender, mut frames) = tokio::sync::mpsc::channel(CONTROL_QUEUE_CAPACITY);
    let reader = tokio::spawn(read_control(receiver, frames_sender));

    let outcome = async {
        send_event(&sender, ProxyControlEvent::Ready { generation }).await?;
        let mut grants = BTreeMap::<ProxyCapability, Vec<(u64, UnixStream)>>::new();
        let mut grant_ids = HashSet::new();

        while let Some(frame) = frames.recv().await {
            let request = decode_proxy_control_request(&frame.bytes)?;
            if request.generation() != generation {
                bail!("proxy control request names a stale generation");
            }
            match request {
                ProxyControlRequest::Attach(grant) => {
                    if frame.fds.len() != 1 {
                        bail!("proxy capability attach requires exactly one descriptor");
                    }
                    let capability = grant.capability();
                    let grant_id = grant.grant_id();
                    let current = grants.get(&capability).map_or(0, Vec::len);
                    let rejection = if grant_ids.contains(&grant_id) {
                        Some(ProxyControlRejection::DuplicateGrant)
                    } else if grant_ids.len() >= GRANT_LIMIT
                        || (capability == ProxyCapability::Traffic && current >= TRAFFIC_GRANT_LIMIT)
                    {
                        Some(ProxyControlRejection::Capacity)
                    } else if capability != ProxyCapability::Traffic && current != 0 {
                        Some(ProxyControlRejection::DuplicateCapability)
                    } else {
                        None
                    };
                    if let Some(reason) = rejection {
                        send_event(
                            &sender,
                            ProxyControlEvent::Rejected {
                                generation,
                                grant_id,
                                reason,
                            },
                        )
                        .await?;
                        continue;
                    }
                    let descriptor = frame.fds.into_iter().next().expect("one checked descriptor");
                    grants
                        .entry(capability)
                        .or_default()
                        .push((grant_id, UnixStream::from(descriptor)));
                    grant_ids.insert(grant_id);
                    send_event(&sender, ProxyControlEvent::Adopted { generation, grant_id }).await?;
                }
                ProxyControlRequest::Shutdown { .. } => {
                    if !frame.fds.is_empty() {
                        bail!("proxy shutdown must not carry descriptors");
                    }
                    break;
                }
            }
        }

        drop(grants);
        send_event(&sender, ProxyControlEvent::Stopped { generation }).await
    }
    .await;

    let _ = shutdown.shutdown(std::net::Shutdown::Both);
    let reader_outcome = reader.await.context("proxy control reader task failed")?;
    outcome?;
    reader_outcome?;
    Ok(())
}

async fn read_control(receiver: ControlReceiver, frames: tokio::sync::mpsc::Sender<ControlFrame>) -> io::Result<()> {
    loop {
        match receiver.recv().await {
            Ok(frame) => {
                if frames.send(frame).await.is_err() {
                    return Ok(());
                }
            }
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(error) => return Err(error),
        }
    }
}

async fn send_event(sender: &ControlSender, event: ProxyControlEvent) -> Result<()> {
    tokio::time::timeout(
        CONTROL_SEND_TIMEOUT,
        sender.send(&encode_proxy_control_event(event), &[]),
    )
    .await
    .context("proxy control event send timed out")??;
    Ok(())
}

fn parse_generation(value: &str) -> std::result::Result<ProxyGeneration, String> {
    if value.len() != 32
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err("generation must be 32 lowercase hexadecimal digits".into());
    }
    let mut bytes = [0; 16];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).map_err(|_| "invalid generation")?;
    }
    let generation = ProxyGeneration::new(bytes);
    if generation.as_bytes() == [0; 16] {
        return Err("generation must not be zero".into());
    }
    Ok(generation)
}

#[cfg(test)]
mod tests;
