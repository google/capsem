use std::os::fd::AsFd as _;
use std::sync::Arc;

use capsem_proto::ledger::LedgerClientRole;
use capsem_proto::ledger_control::{
    decode_ledger_control_request, encode_ledger_control_event, LedgerClientCloseReason,
};

use super::*;

const GENERATION: LedgerGeneration = LedgerGeneration::new([7; 16]);

fn decode_generation(value: &str) -> LedgerGeneration {
    let mut bytes = [0; 16];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).unwrap();
    }
    LedgerGeneration::new(bytes)
}

#[tokio::test]
async fn fake_ledger_worker_child() {
    let Ok(mode) = std::env::var("CAPSEM_FAKE_LEDGER_WORKER") else {
        return;
    };
    if mode == "refuse" {
        std::process::exit(42);
    }
    let generation = decode_generation(&std::env::var("CAPSEM_FAKE_LEDGER_GENERATION").unwrap());
    let announced = if mode == "stale_ready" {
        LedgerGeneration::new([8; 16])
    } else {
        generation
    };
    let control = UnixStream::from(capsem_foundation::unix::fd::duplicate(std::io::stdin().as_fd()).unwrap());
    let control_tx = Arc::new(ControlSender::new(control.try_clone().unwrap()).unwrap());
    let control_rx = ControlReceiver::new(control).unwrap();
    control_tx
        .send(
            &encode_ledger_control_event(LedgerControlEvent::Ready { generation: announced }),
            &[],
        )
        .await
        .unwrap();
    if mode == "stale_ready" {
        std::future::pending::<()>().await;
    }

    let mut held = Vec::new();
    loop {
        let frame = control_rx.recv().await.unwrap();
        match decode_ledger_control_request(&frame.bytes).unwrap() {
            LedgerControlRequest::Attach(grant) => {
                if mode == "die_with_attach_pending" {
                    std::process::exit(43);
                }
                let stream = UnixStream::from(frame.fds.into_iter().next().unwrap());
                control_tx
                    .send(
                        &encode_ledger_control_event(LedgerControlEvent::Adopted {
                            generation,
                            client_id: grant.client_id(),
                        }),
                        &[],
                    )
                    .await
                    .unwrap();
                if mode == "close_clients" {
                    drop(stream);
                    control_tx
                        .send(
                            &encode_ledger_control_event(LedgerControlEvent::Closed {
                                generation,
                                client_id: grant.client_id(),
                                reason: LedgerClientCloseReason::Disconnected,
                            }),
                            &[],
                        )
                        .await
                        .unwrap();
                } else {
                    held.push(stream);
                }
            }
            LedgerControlRequest::Shutdown { .. } if mode == "stall_shutdown" => {
                std::future::pending::<()>().await;
            }
            LedgerControlRequest::Shutdown { .. } => {
                control_tx
                    .send(
                        &encode_ledger_control_event(LedgerControlEvent::Stopped { generation }),
                        &[],
                    )
                    .await
                    .unwrap();
                return;
            }
        }
    }
}

async fn fake(mode: &'static str) -> Result<LedgerWorker> {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("session.db");
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "ledger_worker::tests::fake_ledger_worker_child",
            "--nocapture",
        ])
        .env_clear()
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    launch(command, &database, GENERATION, move |command, generation, _database| {
        command
            .env("CAPSEM_FAKE_LEDGER_WORKER", mode)
            .env("CAPSEM_FAKE_LEDGER_GENERATION", generation_hex(generation));
    })
    .await
}

#[tokio::test]
async fn supervisor_mints_unique_role_bound_channels_and_reaps() {
    let worker = fake("normal").await.unwrap();
    assert_eq!(worker.generation(), GENERATION);
    let mut stopped = worker.stop_receiver();
    let reader = worker.connect(LedgerClientRole::Reader).await.unwrap();
    let producer = worker.connect(LedgerClientRole::VmOwner).await.unwrap();
    assert_eq!(reader.grant().generation(), GENERATION);
    assert_eq!(reader.grant().role(), LedgerClientRole::Reader);
    assert_eq!(producer.grant().role(), LedgerClientRole::VmOwner);
    assert_ne!(reader.grant().client_id(), producer.grant().client_id());
    let (reader_stream, reader_grant) = reader.into_parts();
    assert_eq!(reader_grant.role(), LedgerClientRole::Reader);
    drop(reader_stream);
    drop(producer);
    worker.shutdown().await.unwrap();
    stopped.changed().await.unwrap();
    assert!(stopped.borrow().as_deref().unwrap().contains("coordinator"));
}

#[test]
fn generated_worker_lifetimes_are_fresh_and_nonzero() {
    assert_ne!(fresh_generation(), fresh_generation());
}

#[tokio::test]
async fn client_close_events_do_not_kill_the_worker_supervisor() {
    let worker = fake("close_clients").await.unwrap();
    drop(worker.connect(LedgerClientRole::Reader).await.unwrap());
    tokio::task::yield_now().await;
    let next = worker.connect(LedgerClientRole::Maintainer).await.unwrap();
    assert_eq!(next.grant().client_id(), 2);
    worker.shutdown().await.unwrap();
}

#[tokio::test]
async fn startup_refusal_and_stale_readiness_never_return_authority() {
    for mode in ["refuse", "stale_ready"] {
        let error = fake(mode).await.err().expect("invalid startup must fail");
        let diagnostic = format!("{error:#}");
        assert!(
            diagnostic.contains("before readiness") || diagnostic.contains("stale generation"),
            "{diagnostic}"
        );
    }
}

#[tokio::test]
async fn death_fails_current_and_queued_channel_grants_explicitly() {
    let worker = fake("die_with_attach_pending").await.unwrap();
    let first = worker.clone();
    let second = worker.clone();
    let (first, second) = tokio::join!(
        first.connect(LedgerClientRole::Reader),
        second.connect(LedgerClientRole::Maintainer)
    );
    for result in [first, second] {
        let error = result.expect_err("worker death must fail every pending grant");
        let diagnostic = format!("{error:#}");
        assert!(diagnostic.contains("ledger worker"), "{diagnostic}");
    }
}

#[tokio::test]
async fn stalled_shutdown_is_killed_and_reaped_within_the_bound() {
    let worker = fake("stall_shutdown").await.unwrap();
    let started = std::time::Instant::now();
    let error = worker.shutdown().await.unwrap_err();
    assert!(format!("{error:#}").contains("bounded shutdown timed out"), "{error:#}");
    assert!(started.elapsed() < Duration::from_secs(2));
}
