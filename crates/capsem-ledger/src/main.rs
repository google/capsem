use std::collections::HashSet;
use std::io;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use capsem_foundation::unix::{fd, router_channel};
use capsem_logger::ledger_server::{LedgerClientExit, LedgerServer};
use capsem_proto::ledger::LedgerGeneration;
use capsem_proto::ledger_control::{
    decode_ledger_control_request, encode_ledger_control_event, LedgerClientCloseReason, LedgerControlEvent,
    LedgerControlRejection, LedgerControlRequest, LEDGER_CONTROL_FRAME_SIZE, LEDGER_CONTROL_MAX_FDS,
};
use clap::Parser;
use tokio::task::{JoinError, JoinSet};

const CLIENT_LIMIT: usize = 64;

type ControlSender = router_channel::DescriptorSender<LEDGER_CONTROL_FRAME_SIZE, LEDGER_CONTROL_MAX_FDS>;
type ControlReceiver = router_channel::DescriptorReceiver<LEDGER_CONTROL_FRAME_SIZE, LEDGER_CONTROL_MAX_FDS>;
type ClientResult = (u64, Result<LedgerClientExit, String>);

#[derive(Parser)]
#[command(version, about = "Dedicated owner of one Capsem session ledger")]
struct Args {
    #[arg(long)]
    parent_pid: u32,
    #[arg(long)]
    database: PathBuf,
    /// Sixteen-byte worker generation as 32 lowercase hexadecimal digits.
    #[arg(long, value_parser = parse_generation)]
    generation: LedgerGeneration,
}

fn main() -> Result<()> {
    let _telemetry = capsem_foundation::telemetry::init(capsem_foundation::telemetry::TelemetryConfig {
        service: "capsem-ledger",
        sink: capsem_foundation::telemetry::LogSink::Stderr,
        default_filter: "capsem_ledger=info,capsem_logger=info,capsem_foundation=warn",
    })?;
    let args = Args::parse();
    capsem_guard::watch_parent_or_exit(Some(args.parent_pid))?;
    let stdin = io::stdin();
    let control = UnixStream::from(fd::duplicate(stdin.as_fd())?);
    let server = Arc::new(LedgerServer::open(&args.database).context("open session ledger")?);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    runtime.block_on(run_control(control, server, args.generation, CLIENT_LIMIT))
}

async fn run_control(
    control: UnixStream,
    server: Arc<LedgerServer>,
    generation: LedgerGeneration,
    client_limit: usize,
) -> Result<()> {
    let shutdown = control.try_clone()?;
    let sender = ControlSender::new(control.try_clone()?)?;
    let receiver = ControlReceiver::new(control)?;
    let (frames_sender, mut frames) = tokio::sync::mpsc::unbounded_channel();
    let reader = tokio::spawn(read_control(receiver, frames_sender));

    let outcome = async {
        send_event(&sender, LedgerControlEvent::Ready { generation }).await?;

        let mut clients = JoinSet::<ClientResult>::new();
        let mut active_ids = HashSet::new();
        let mut seen_ids = HashSet::new();
        loop {
            let next = if clients.is_empty() {
                ControlInput::Frame(frames.recv().await.context("ledger control channel closed")?)
            } else {
                tokio::select! {
                    frame = frames.recv() => ControlInput::Frame(frame.context("ledger control channel closed")?),
                    result = clients.join_next() => ControlInput::Client(result.expect("nonempty ledger client set")),
                }
            };
            match next {
                ControlInput::Client(result) => {
                    let (event, shutdown) = client_finished(result, generation, &mut active_ids)?;
                    send_event(&sender, event).await?;
                    if shutdown {
                        break;
                    }
                }
                ControlInput::Frame(frame) => {
                    let request = decode_ledger_control_request(&frame.bytes)?;
                    if request.generation() != generation {
                        bail!("ledger control request names a stale generation");
                    }
                    match request {
                        LedgerControlRequest::Attach(grant) => {
                            if frame.fds.len() != 1 {
                                bail!("ledger attach requires exactly one descriptor");
                            }
                            let client_id = grant.client_id();
                            let rejected = if seen_ids.contains(&client_id) {
                                Some(LedgerControlRejection::DuplicateClient)
                            } else if active_ids.len() >= client_limit {
                                Some(LedgerControlRejection::Capacity)
                            } else {
                                None
                            };
                            if let Some(reason) = rejected {
                                send_event(
                                    &sender,
                                    LedgerControlEvent::Rejected {
                                        generation,
                                        client_id,
                                        reason,
                                    },
                                )
                                .await?;
                                continue;
                            }
                            let descriptor = frame.fds.into_iter().next().expect("one checked descriptor");
                            let stream = UnixStream::from(descriptor);
                            let client_server = Arc::clone(&server);
                            active_ids.insert(client_id);
                            seen_ids.insert(client_id);
                            clients.spawn(async move { (client_id, client_server.serve_client(stream, grant).await) });
                            send_event(&sender, LedgerControlEvent::Adopted { generation, client_id }).await?;
                        }
                        LedgerControlRequest::Shutdown { .. } => {
                            if !frame.fds.is_empty() {
                                bail!("ledger shutdown must not carry descriptors");
                            }
                            break;
                        }
                    }
                }
            }
        }

        clients.abort_all();
        while let Some(result) = clients.join_next().await {
            if let Ok((client_id, _)) = result {
                active_ids.remove(&client_id);
            }
        }
        drop(active_ids);
        drop(seen_ids);
        let server = Arc::try_unwrap(server).map_err(|_| anyhow::anyhow!("ledger client owner survived shutdown"))?;
        drop(server);
        send_event(&sender, LedgerControlEvent::Stopped { generation }).await
    }
    .await;

    let _ = shutdown.shutdown(std::net::Shutdown::Both);
    let reader_outcome = reader.await.context("ledger control reader task failed")?;
    outcome?;
    reader_outcome?;
    Ok(())
}

async fn read_control(
    receiver: ControlReceiver,
    frames: tokio::sync::mpsc::UnboundedSender<router_channel::DescriptorFrame<LEDGER_CONTROL_FRAME_SIZE>>,
) -> io::Result<()> {
    loop {
        match receiver.recv().await {
            Ok(frame) => {
                if frames.send(frame).is_err() {
                    return Ok(());
                }
            }
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(error) => return Err(error),
        }
    }
}

enum ControlInput {
    Frame(router_channel::DescriptorFrame<LEDGER_CONTROL_FRAME_SIZE>),
    Client(std::result::Result<ClientResult, JoinError>),
}

fn client_finished(
    result: std::result::Result<ClientResult, JoinError>,
    generation: LedgerGeneration,
    active_ids: &mut HashSet<u64>,
) -> Result<(LedgerControlEvent, bool)> {
    let (client_id, result) = result.context("ledger client task failed")?;
    active_ids.remove(&client_id);
    let (reason, shutdown) = match result {
        Ok(LedgerClientExit::Disconnected) => (LedgerClientCloseReason::Disconnected, false),
        Ok(LedgerClientExit::ShutdownRequested) => (LedgerClientCloseReason::ShutdownRequested, true),
        Err(error) => {
            tracing::warn!(client_id, %error, "ledger client stopped");
            (LedgerClientCloseReason::ProtocolError, false)
        }
    };
    Ok((
        LedgerControlEvent::Closed {
            generation,
            client_id,
            reason,
        },
        shutdown,
    ))
}

async fn send_event(sender: &ControlSender, event: LedgerControlEvent) -> Result<()> {
    sender.send(&encode_ledger_control_event(event), &[]).await?;
    Ok(())
}

fn parse_generation(value: &str) -> std::result::Result<LedgerGeneration, String> {
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
    let generation = LedgerGeneration::new(bytes);
    if generation.as_bytes() == [0; 16] {
        return Err("generation must not be zero".into());
    }
    Ok(generation)
}

#[cfg(test)]
mod tests;
