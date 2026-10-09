use std::os::fd::AsFd as _;
use std::sync::Arc;

use capsem_proto::ledger::{LedgerChannelGrant, LedgerClientRole, LedgerGeneration};
use capsem_proto::proxy_control::{decode_proxy_control_request, encode_proxy_control_event};

use super::*;

const GENERATION: ProxyGeneration = ProxyGeneration::new([9; 16]);
const ACTIVE_POLICY: &[u8] = br#"
[network]
[user_rules.profiles.rules.worker_http]
name = "worker_http"
action = "allow"
match = 'http.host == "worker.example"'
[corp_rules]
"#;

fn decode_generation(value: &str) -> ProxyGeneration {
    let mut bytes = [0; 16];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).unwrap();
    }
    ProxyGeneration::new(bytes)
}

#[test]
fn standalone_worker_command_carries_trusted_provider_selection() {
    let mut command = Command::new("capsem-proxy");
    configure_worker_command(
        &mut command,
        GENERATION,
        &ProxyWorkerMode::Standalone("openai".to_string()),
    );
    let args = command
        .as_std()
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect::<Vec<_>>();

    assert_eq!(
        args,
        vec![
            "--parent-pid".to_string(),
            std::process::id().to_string(),
            "--generation".to_string(),
            "09090909090909090909090909090909".to_string(),
            "--standalone-provider".to_string(),
            "openai".to_string(),
        ]
    );
}

#[tokio::test]
async fn fake_proxy_worker_child() {
    let Ok(mode) = std::env::var("CAPSEM_FAKE_PROXY_WORKER") else {
        return;
    };
    if mode == "refuse" {
        std::process::exit(42);
    }
    let generation = decode_generation(&std::env::var("CAPSEM_FAKE_PROXY_GENERATION").unwrap());
    let control = UnixStream::from(capsem_foundation::unix::fd::duplicate(std::io::stdin().as_fd()).unwrap());
    let control_tx = Arc::new(ControlSender::new(control.try_clone().unwrap()).unwrap());
    let control_rx = ControlReceiver::new(control).unwrap();
    control_tx
        .send(
            &encode_proxy_control_event(ProxyControlEvent::Ready { generation }),
            &[],
        )
        .await
        .unwrap();

    let frame = control_rx.recv().await.unwrap();
    let ProxyControlRequest::Attach(grant) = decode_proxy_control_request(&frame.bytes).unwrap() else {
        panic!("expected policy attach")
    };
    assert_eq!(grant.generation(), generation);
    assert_eq!(grant.capability(), ProxyCapability::Policy);
    let policy_stream = UnixStream::from(frame.fds.into_iter().next().unwrap());
    control_tx
        .send(
            &encode_proxy_control_event(ProxyControlEvent::Adopted {
                generation,
                grant_id: grant.grant_id(),
            }),
            &[],
        )
        .await
        .unwrap();
    let (policy_tx, policy_rx) =
        ipc_channel::channel_from_std::<ProxyPolicyResponse, ProxyPolicyRequest>(policy_stream).unwrap();
    let (control_events_tx, mut control_events) = mpsc::channel(COMMAND_QUEUE_CAPACITY);
    tokio::spawn(read_control(control_rx, control_events_tx));
    let mut policies = 0;
    let mut held = Vec::new();
    loop {
        tokio::select! {
            request = policy_rx.recv() => {
                let request = request.unwrap();
                policies += 1;
                if mode == "die_with_policy_pending" && policies == 2 {
                    std::process::exit(43);
                }
                let ProxyPolicyRequest::Apply { request_id, active_policy, .. } = request;
                let response = match capsem_core::net::proxy_engine::ProxyRuntimePolicy::compile(&active_policy) {
                    Ok(runtime) => ProxyPolicyResponse::Applied {
                        request_id,
                        active_policy_digest: runtime.snapshot().digest().to_string(),
                    },
                    Err(error) => ProxyPolicyResponse::rejected(request_id, error),
                };
                policy_tx.send(response).await.unwrap();
            }
            frame = control_events.recv() => {
                let frame = frame.unwrap().unwrap();
                match decode_proxy_control_request(&frame.bytes).unwrap() {
                    ProxyControlRequest::Attach(grant) => {
                        if mode == "ledger" {
                            assert_eq!(grant.capability(), ProxyCapability::Ledger);
                            assert_eq!(grant.ledger_grant(), Some(proxy_ledger_grant()));
                        }
                        held.extend(frame.fds.into_iter().map(UnixStream::from));
                        control_tx.send(
                            &encode_proxy_control_event(ProxyControlEvent::Adopted {
                                generation,
                                grant_id: grant.grant_id(),
                            }),
                            &[],
                        ).await.unwrap();
                        if mode == "traffic_close" {
                            assert_eq!(grant.capability(), ProxyCapability::HttpTraffic);
                            control_tx.send(
                                &encode_proxy_control_event(ProxyControlEvent::Closed {
                                    generation,
                                    grant_id: grant.grant_id(),
                                    reason: capsem_proto::proxy_control::ProxyChannelCloseReason::Disconnected,
                                }),
                                &[],
                            ).await.unwrap();
                        }
                    }
                    ProxyControlRequest::Shutdown { .. } if mode == "stall_shutdown" => {
                        std::future::pending::<()>().await;
                    }
                    ProxyControlRequest::Shutdown { .. } => {
                        control_tx.send(
                            &encode_proxy_control_event(ProxyControlEvent::Stopped { generation }),
                            &[],
                        ).await.unwrap();
                        return;
                    }
                }
            }
        }
    }
}

fn proxy_ledger_grant() -> LedgerChannelGrant {
    LedgerChannelGrant::new(LedgerGeneration::new([3; 16]), 29, LedgerClientRole::Proxy).unwrap()
}

pub(crate) async fn fake(mode: &'static str) -> Result<ProxyWorker> {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "proxy_worker::tests::fake_proxy_worker_child", "--nocapture"])
        .env_clear()
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    launch(
        command,
        GENERATION,
        ACTIVE_POLICY.to_vec(),
        move |command, generation| {
            command
                .env("CAPSEM_FAKE_PROXY_WORKER", mode)
                .env("CAPSEM_FAKE_PROXY_GENERATION", generation_hex(generation));
        },
    )
    .await
}

#[tokio::test]
async fn supervisor_applies_policy_grants_connected_descriptors_and_reaps() {
    let worker = fake("normal").await.unwrap();
    assert_eq!(worker.generation(), GENERATION);
    let digest = worker.apply_policy(ACTIVE_POLICY.to_vec()).await.unwrap();
    assert_eq!(
        digest,
        capsem_core::net::policy_config::active_policy_digest(ACTIVE_POLICY)
    );
    let (peer, granted) = UnixStream::pair().unwrap();
    worker.grant(ProxyCapability::Telemetry, granted.into()).await.unwrap();
    drop(peer);
    worker.shutdown().await.unwrap();
}

#[tokio::test]
async fn ordinary_traffic_disconnect_does_not_terminate_the_proxy_generation() {
    let worker = fake("traffic_close").await.unwrap();
    let (peer, granted) = UnixStream::pair().unwrap();
    worker
        .grant(ProxyCapability::HttpTraffic, granted.into())
        .await
        .unwrap();
    drop(peer);
    tokio::task::yield_now().await;

    let digest = worker.apply_policy(ACTIVE_POLICY.to_vec()).await.unwrap();
    assert_eq!(
        digest,
        capsem_core::net::policy_config::active_policy_digest(ACTIVE_POLICY)
    );
    worker.shutdown().await.unwrap();
}

#[tokio::test]
async fn ledger_descriptor_carries_its_exact_proxy_authority() {
    let worker = fake("ledger").await.unwrap();
    let (peer, granted) = UnixStream::pair().unwrap();
    let (commitment_peer, commitment) = UnixStream::pair().unwrap();
    worker
        .grant_ledger(granted, commitment, proxy_ledger_grant())
        .await
        .unwrap();
    drop(peer);
    drop(commitment_peer);
    worker.shutdown().await.unwrap();
}

#[tokio::test]
async fn startup_refusal_never_returns_a_worker_handle() {
    let error = fake("refuse").await.err().expect("refused startup must fail");
    assert!(format!("{error:#}").contains("exited before readiness"), "{error:#}");
}

#[tokio::test]
async fn death_fails_current_and_queued_policy_requests_explicitly() {
    let worker = fake("die_with_policy_pending").await.unwrap();
    let first = worker.clone();
    let second = worker.clone();
    let (first, second) = tokio::join!(
        first.apply_policy(ACTIVE_POLICY.to_vec()),
        second.apply_policy(ACTIVE_POLICY.to_vec())
    );
    for result in [first, second] {
        let error = result.expect_err("worker death must fail policy request");
        let diagnostic = format!("{error:#}");
        assert!(
            diagnostic.contains("proxy worker") && (diagnostic.contains("exited") || diagnostic.contains("stopped")),
            "{diagnostic}"
        );
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
async fn unexpected_proxy_death_terminates_only_its_matching_vm_generation() {
    let state = crate::tests::make_test_state();
    let mut owner_a = Command::new("sleep").arg("30").kill_on_drop(true).spawn().unwrap();
    let mut owner_b = Command::new("sleep").arg("30").kill_on_drop(true).spawn().unwrap();
    let session_a = state.run_dir.join("sessions/a");
    let session_b = state.run_dir.join("sessions/b");
    std::fs::create_dir_all(&session_a).unwrap();
    std::fs::create_dir_all(&session_b).unwrap();
    crate::tests::insert_fake_instance_with_session_dir(&state, "a", owner_a.id().unwrap(), session_a);
    crate::tests::insert_fake_instance_with_session_dir(&state, "b", owner_b.id().unwrap(), session_b);
    let generation = state.instances.lock().unwrap()["a"].generation;
    let worker = fake("die_with_policy_pending").await.unwrap();
    let caller = worker.clone();
    state.register_proxy_worker("a", generation, worker).unwrap();

    assert!(caller.apply_policy(ACTIVE_POLICY.to_vec()).await.is_err());
    let status = tokio::time::timeout(Duration::from_secs(2), owner_a.wait())
        .await
        .expect("matching VM owner was not terminated")
        .unwrap();
    assert!(!status.success());
    assert!(
        owner_b.try_wait().unwrap().is_none(),
        "unrelated VM owner was terminated"
    );

    state.evict_instance("a", generation);
    let generation_b = state.instances.lock().unwrap()["b"].generation;
    state.evict_instance("b", generation_b);
    owner_b.start_kill().unwrap();
    owner_b.wait().await.unwrap();
}
