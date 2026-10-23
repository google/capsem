use std::os::fd::AsFd as _;
use std::os::unix::fs::PermissionsExt as _;
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
    // Lifecycle-only modes keep raw descriptors. Storage acceptance uses the
    // real ledger protocol, role checks, writer and archive owner.
    let storage = (mode == "storage").then(|| {
        Arc::new(
            capsem_logger::ledger_server::LedgerServer::open(&PathBuf::from(
                std::env::var("CAPSEM_FAKE_LEDGER_DATABASE").unwrap(),
            ))
            .unwrap(),
        )
    });
    let mut storage_clients = tokio::task::JoinSet::new();
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
                if let Some(server) = &storage {
                    let server = Arc::clone(server);
                    storage_clients.spawn(async move { server.serve_client(stream, grant).await.unwrap() });
                } else if mode == "close_clients" {
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
                // Match the production coordinator: shutdown revokes client
                // tasks, rather than waiting for every peer to close first.
                storage_clients.abort_all();
                while let Some(client) = storage_clients.join_next().await {
                    if let Err(error) = client {
                        assert!(error.is_cancelled(), "{error}");
                    }
                }
                drop(storage);
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
    let mut command = crate::tests::isolated_test_command(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "ledger_worker::tests::fake_ledger_worker_child",
            "--nocapture",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    // This fixture owns only the control protocol, not a real archive flush.
    // Storage fixtures spawned through LedgerWorkers keep the production bound.
    launch(
        command,
        &database,
        GENERATION,
        Duration::from_millis(200),
        move |command, generation, _database| {
            command
                .env("CAPSEM_FAKE_LEDGER_WORKER", mode)
                .env("CAPSEM_FAKE_LEDGER_GENERATION", generation_hex(generation));
        },
    )
    .await
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

pub(crate) fn fake_binary(directory: &Path, mode: &str) -> PathBuf {
    let binary = directory.join(format!("capsem-ledger-{mode}"));
    write_fake_binary(&binary, mode);
    binary
}

fn write_fake_binary(binary: &Path, mode: &str) {
    let test_executable = std::env::current_exe().unwrap();
    let generations = binary.with_extension("generations");
    let script = format!(
        "#!/bin/sh\ngeneration=\ndatabase=\nwhile [ \"$#\" -gt 0 ]; do\n  case \"$1\" in\n    --generation) shift; generation=$1 ;;\n    --database) shift; database=$1 ;;\n  esac\n  shift\ndone\nprintf '%s\\n' \"$generation\" >> {}\nCAPSEM_FAKE_LEDGER_WORKER={} CAPSEM_FAKE_LEDGER_GENERATION=\"$generation\" CAPSEM_FAKE_LEDGER_DATABASE=\"$database\" exec {} --exact ledger_worker::tests::fake_ledger_worker_child --nocapture\n",
        shell_quote(&generations.to_string_lossy()),
        shell_quote(mode),
        shell_quote(&test_executable.to_string_lossy()),
    );
    std::fs::write(binary, script).unwrap();
    let mut permissions = std::fs::metadata(binary).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(binary, permissions).unwrap();
}

fn session_paths(root: &Path, name: &str) -> (PathBuf, PathBuf) {
    let directory = root.join(name);
    std::fs::create_dir(&directory).unwrap();
    (directory.join("session.db"), directory.join("ledger.log"))
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

#[tokio::test]
async fn terminal_event_after_observed_child_exit_is_still_drained() {
    let mut command = crate::tests::isolated_test_command(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "ledger_worker::tests::fake_ledger_worker_child",
            "--nocapture",
        ])
        .env("CAPSEM_FAKE_LEDGER_WORKER", "refuse")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let mut child = command.spawn().unwrap();
    assert_eq!(child.wait().await.unwrap().code(), Some(42));

    let (control, _control_peer) = UnixStream::pair().unwrap();
    let (events_tx, control_events) = mpsc::channel(1);
    let mut process = WorkerProcess {
        shutdown_timeout: Duration::from_millis(200),
        generation: GENERATION,
        child,
        control_tx: ControlSender::new(control).unwrap(),
        control_events,
        control_reader: None,
        next_client_id: 1,
    };
    let delayed = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(10)).await;
        events_tx
            .send(Ok(ControlFrame {
                bytes: encode_ledger_control_event(LedgerControlEvent::Stopped { generation: GENERATION }),
                fds: Vec::new(),
            }))
            .await
            .unwrap();
    });

    assert!(matches!(
        process.recv_event().await.unwrap(),
        LedgerControlEvent::Stopped { generation } if generation == GENERATION
    ));
    delayed.await.unwrap();
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

#[tokio::test]
async fn lifecycle_slot_serializes_concurrent_leases_and_replacement() {
    let directory = tempfile::tempdir().unwrap();
    let binary = fake_binary(directory.path(), "normal");
    let workers = Arc::new(LedgerWorkers::new(binary, None));
    let (database, log) = session_paths(directory.path(), "session-a");

    let (reader, maintainer) = tokio::join!(
        workers.acquire("a", &database, &log, LedgerClientRole::Reader),
        workers.acquire("a", &database, &log, LedgerClientRole::Maintainer),
    );
    let reader = reader.unwrap();
    let maintainer = maintainer.unwrap();
    assert_eq!(reader.grant().generation(), maintainer.grant().generation());
    assert_ne!(reader.grant().client_id(), maintainer.grant().client_id());
    let first_generation = reader.grant().generation();

    let (other_database, other_log) = session_paths(directory.path(), "rebound");
    let error = workers
        .acquire("a", &other_database, &other_log, LedgerClientRole::Reader)
        .await
        .unwrap_err();
    assert!(format!("{error:#}").contains("owns"), "{error:#}");

    drop(reader);
    drop(maintainer);
    workers.shutdown("a").await.unwrap();
    assert_eq!(workers.generation("a").await, None);
    let replacement = workers
        .acquire("a", &database, &log, LedgerClientRole::Reader)
        .await
        .unwrap();
    assert_ne!(replacement.grant().generation(), first_generation);
    drop(replacement);
    workers.shutdown("a").await.unwrap();
}

#[tokio::test]
async fn stopping_one_session_leaves_an_unrelated_session_generation_alive() {
    let directory = tempfile::tempdir().unwrap();
    let binary = fake_binary(directory.path(), "normal");
    let workers = Arc::new(LedgerWorkers::new(binary, None));
    let (database_a, log_a) = session_paths(directory.path(), "session-a");
    let (database_b, log_b) = session_paths(directory.path(), "session-b");
    let client_a = workers
        .acquire("a", &database_a, &log_a, LedgerClientRole::Reader)
        .await
        .unwrap();
    let client_b = workers
        .acquire("b", &database_b, &log_b, LedgerClientRole::Reader)
        .await
        .unwrap();
    let generation_b = client_b.grant().generation();

    drop(client_a);
    workers.shutdown("a").await.unwrap();
    let second_b = workers
        .acquire("b", &database_b, &log_b, LedgerClientRole::Maintainer)
        .await
        .unwrap();
    assert_eq!(second_b.grant().generation(), generation_b);

    drop(client_b);
    drop(second_b);
    workers.shutdown_all().await;
    assert_eq!(workers.generation("b").await, None);
}

#[tokio::test]
async fn shutdown_applies_retention_without_granting_producers_maintenance() {
    for (policy, expires) in [(zero_days as fn() -> u64, true), (thirty_days as fn() -> u64, false)] {
        retention_case(policy, expires).await;
    }
}

fn zero_days() -> u64 {
    0
}

fn thirty_days() -> u64 {
    30
}

async fn retention_case(policy: fn() -> u64, expires: bool) {
    let directory = tempfile::tempdir().unwrap();
    let workers = Arc::new(LedgerWorkers::new(
        fake_binary(directory.path(), "storage"),
        Some(policy),
    ));
    let (database, log) = session_paths(directory.path(), "retained");
    let (stream, grant) = workers
        .acquire("retained", &database, &log, LedgerClientRole::VmOwner)
        .await
        .unwrap()
        .into_parts();
    let producer = capsem_logger::DbWriter::from_ledger_channel(stream, grant, &database, 16).unwrap();
    producer
        .write_checked(capsem_logger::WriteOp::SecurityRuleEvent(
            capsem_logger::SecurityRuleEvent::new(
                1_789_000_223_456,
                "abcdef123450",
                "model.call",
                "profiles.rules.example",
                r#"{"name":"example"}"#,
                r#"{"from":"an old session"}"#,
            ),
        ))
        .await
        .unwrap();
    producer.flush_checked().await.unwrap();
    let denied = producer.retain_bodies_since("2999-01-01T00:00:00Z").await.unwrap_err();
    assert!(denied.contains("UnauthorizedOperation"), "{denied}");

    let (stream, grant) = workers
        .acquire("retained", &database, &log, LedgerClientRole::Reader)
        .await
        .unwrap()
        .into_parts();
    let reader = capsem_logger::ledger_client::LedgerClient::connect(stream, grant, database.clone())
        .await
        .unwrap();
    let expected =
        capsem_proto::forensic::SecurityForensicEvent::from_json(r#"{"from":"an old session"}"#, "model.call")
            .unwrap()
            .encode()
            .unwrap();
    let bodies = reader.read_bodies("abcdef123450").await.unwrap();
    assert_eq!(bodies.len(), 1);
    assert_eq!(
        bodies[0].bytes, expected,
        "a denied producer must not remove archived bytes"
    );
    drop(reader);
    tokio::task::spawn_blocking(move || producer.shutdown_blocking())
        .await
        .unwrap();

    workers.shutdown("retained").await.unwrap();
    let ledger = capsem_logger::DbHandle::open(&database).unwrap();
    let bodies = ledger.read_bodies("abcdef123450").await.unwrap();
    assert_eq!(
        bodies.is_empty(),
        expires,
        "trusted shutdown applies its retention period after the producer drains"
    );
    if !expires {
        assert_eq!(bodies.len(), 1);
        assert_eq!(bodies[0].bytes, expected);
    }
}

#[tokio::test]
async fn crashed_slot_restarts_only_after_reap_with_a_fresh_generation() {
    let directory = tempfile::tempdir().unwrap();
    let binary = fake_binary(directory.path(), "die_with_attach_pending");
    let workers = Arc::new(LedgerWorkers::new(binary.clone(), None));
    let (database, log) = session_paths(directory.path(), "stopped-session");

    let error = workers
        .acquire("stopped", &database, &log, LedgerClientRole::Reader)
        .await
        .unwrap_err();
    assert!(format!("{error:#}").contains("ledger worker"), "{error:#}");
    tokio::time::timeout(Duration::from_secs(2), async {
        while workers.generation("stopped").await.is_some() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("crashed generation was not reaped and removed");

    write_fake_binary(&binary, "normal");
    let reader = workers
        .acquire("stopped", &database, &log, LedgerClientRole::Reader)
        .await
        .unwrap();
    let generations = std::fs::read_to_string(binary.with_extension("generations")).unwrap();
    let generations: Vec<_> = generations.lines().collect();
    assert_eq!(generations.len(), 2);
    assert_ne!(generations[0], generations[1]);
    drop(reader);
    workers.shutdown("stopped").await.unwrap();
}
