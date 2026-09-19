use super::security::{close_reason, AuditFlow};
use super::*;
use crate::security_engine::network::NetworkReason;
use crate::security_engine::{RuntimeSecurityEventType as Type, SecurityEnforcementAction as Action};
use capsem_router::MAX_CONNECTIONS;
use tokio::time::Instant;

struct Active {
    audit: AuditFlow,
    guest: capsem_proto::router::FlowKey,
    _pending: Pending,
    graceful: bool,
    _permit: tokio::sync::OwnedSemaphorePermit,
    setup: tokio::task::AbortHandle,
    source: Arc<Source>,
    connection: Option<VsockConnection>,
    acknowledgement: Option<Instant>,
    accepted: bool,
    close_deadline: Option<Instant>,
    preview: Option<capsem_proto::PreviewAdmissionKind>,
    /// The preview session that admitted this flow; it bounds the flow.
    session: Option<SessionLease>,
    /// Why the broker is ending this flow itself, recorded at its close.
    ending: Option<NetworkReason>,
}
impl Drop for Active {
    fn drop(&mut self) {
        self.setup.abort();
        let shutdown = if self.graceful {
            self.source.close_gracefully()
        } else {
            self.source.reset()
        };
        if let Err(error) = shutdown {
            tracing::debug!(%error, "publication source shutdown");
            if let Err(error) = self.source.close_gracefully() {
                tracing::debug!(%error, "publication source fallback shutdown");
            }
        }
        if let Some(connection) = &self.connection {
            if let Err(error) = connection.shutdown_both() {
                tracing::debug!(%error, "publication destination shutdown");
            }
        }
    }
}

pub(super) async fn serve(
    owner: Arc<Publisher>,
    mut incoming: mpsc::Receiver<Incoming>,
    control: mpsc::Sender<ServiceToProcess>,
    router: Arc<companion::Router>,
    cancellation: CancellationToken,
) -> Result<()> {
    ensure!(owner.security.is_some(), "publication security context missing");
    let (ingress, setup_slots, setup_rate) = (owner.ingress.clone(), owner.setups.clone(), owner.setup_rate.clone());
    let mut active: HashMap<u64, Active> = HashMap::new();
    let mut connecting: HashMap<u64, Active> = HashMap::new();
    let mut setups = tokio::task::JoinSet::new();
    let (queue, mut records) = mpsc::channel(MAX_CONNECTIONS);
    let (guest_close, mut guest_records) = mpsc::channel(MAX_CONNECTIONS);
    let mut acknowledgements = tokio::time::interval(Duration::from_millis(100));
    let result = Box::pin(async {
        loop {
            tokio::select! {
                _ = cancellation.cancelled() => return Ok(()),
                _ = router.closed.cancelled() => anyhow::bail!("VM router closed"),
                arrival = incoming.recv(), if active.len() + connecting.len() < MAX_CONNECTIONS => {
                    let Some(Incoming { source, audit, port: guest_port, target, preview, session }) = arrival else {
                        return Ok(());
                    };
                    let Ok(permit) = ingress.clone().try_acquire_owned() else {
                        tracing::debug!("VM ingress connection quota exhausted");
                        continue;
                    };
                    source.prepare()?;
                    let source = Arc::new(source);
                    let (pending, receiver) = owner.request(&source, guest_close.clone())?;
                    let id = pending.id;
                    let flow = capsem_proto::router::FlowKey { generation: owner.generation.get(), id };
                    let record = audit.clone();
                    let lease = pending.lease.clone();
                    let control = control.clone();
                    let slots = setup_slots.clone();
                    let rate = setup_rate.clone();
                    let setup = setups.spawn(async move {
                        let mut reason = NetworkReason::Io;
                        let result = tokio::time::timeout(Duration::from_secs(8), async {
                            let _setup = slots.acquire_owned().await.context("guest setup admission closed")?;
                            rate.acquire().await;
                            let action = record.authorize().await?;
                            // Each failure below names what actually happened;
                            // `stale_generation` belongs only to the recheck after
                            // setup, and once covered every path from here.
                            reason = match action {
                                Action::Block => NetworkReason::Blocked,
                                Action::Ask => NetworkReason::ApprovalRequired,
                                Action::Allow => NetworkReason::Unreachable,
                            };
                            ensure!(action == Action::Allow, "publication security decision: {action:?}");
                            let lease = lease.context("guest control lease missing")?;
                            reason = NetworkReason::Cancelled;
                            ensure!(!lease.is_cancelled(), "guest control lease expired");
                            tokio::select! {
                                biased;
                                _ = lease.cancelled() => anyhow::bail!("guest control lease expired"),
                                result = async {
                                    control.send(ServiceToProcess::ConnectPort { flow, port: guest_port, target }).await
                                        .context("guest control closed")?;
                                    receiver.await.context("guest connection cancelled")?
                                } => {
                                    reason = NetworkReason::Refused;
                                    result
                                },
                            }
                        }).await;
                        let result = match result {
                            Ok(result) => result,
                            Err(error) => {
                                reason = NetworkReason::SetupTimeout;
                                Err(anyhow::Error::new(error).context("guest connection timed out"))
                            }
                        };
                        (id, result, reason)
                    });
                    tracing::debug!(connection_id = id, guest_port, "publication connection accepted");
                    connecting.insert(id, Active { audit, guest: flow, _pending: pending, graceful: false, _permit: permit, setup, source, connection: None, acknowledgement: None, accepted: false, close_deadline: None, preview, session, ending: None });
                }
                Some((guest, report)) = guest_records.recv() => {
                    if report.reason != capsem_proto::router::CloseReason::Complete {
                        // report_close already revoked TCP before its control ACK.
                        if let Some((&id, flow)) = active.iter_mut().find(|(_, flow)| flow.guest == guest) {
                            flow.close_deadline = Some(Instant::now() + Duration::from_secs(2));
                            router.cancel(id).await?;
                        }
                    }
                }
                event = records.recv() => match event.context("router event reader closed")? {
                    Event::Accepted(id) => {
                        let audit = {
                            let flow = active.get_mut(&id).context("router acknowledged unknown connection")?;
                            ensure!(flow.acknowledgement.take().is_some() && !flow.accepted, "invalid router acknowledgement");
                            flow.accepted = true;
                            flow.audit.clone()
                        };
                        audit.record(Type::NetworkConnectResult, NetworkReason::Connected, 0, 0).await?;
                    }
                    Event::Closed(id, report) => {
                        ensure!(active.get(&id).is_some_and(|flow| flow.accepted), "router closed unacknowledged connection");
                        let mut flow = active.remove(&id).unwrap();
                        flow.graceful = report.reason == capsem_proto::router::CloseReason::Complete;
                        tracing::debug!(connection_id = id, reason = ?report.reason,
                            from_source = report.from_source, to_source = report.to_source, "publication closed");
                        let guest = flow.guest;
                        let audit = flow.audit.clone();
                        let reason = flow.ending.unwrap_or_else(|| close_reason(report.reason));
                        drop(flow);
                        audit.record(Type::NetworkClose, reason, report.from_source, report.to_source).await?;
                        if report.reason != capsem_proto::router::CloseReason::Complete {
                            abort_guest(&control, vec![guest]).await?;
                        }
                    }
                    Event::Refused(id) => {
                        ensure!(active.get(&id).is_some_and(|flow| flow.acknowledgement.is_some() && !flow.accepted), "unexpected router refusal");
                        let flow = active.remove(&id).unwrap();
                        let guest = flow.guest;
                        let audit = flow.audit.clone();
                        drop(flow);
                        abort_guest(&control, vec![guest]).await?;
                        audit.record(Type::NetworkConnectResult, NetworkReason::Refused, 0, 0).await?;
                    }
                    Event::Ready | Event::ConfinementFailed => anyhow::bail!("unexpected router startup event"),
                    Event::PortClosed(port, _) => anyhow::bail!("pair relay reported a switch port {port}"),
                },
                completed = setups.join_next(), if !setups.is_empty() => {
                    let (id, result, reason) = completed.unwrap().context("publication setup task failed")?;
                    let mut flow = connecting.remove(&id).context("completed unknown publication setup")?;
                    match result {
                        Ok(connection) => {
                            // A session that ended while the guest leg was being
                            // set up admits nothing: refuse rather than grant.
                            if let Some(reason) = flow.session.as_ref().and_then(|session| session.ended(std::time::Instant::now())) {
                                let guest = flow.guest;
                                let audit = flow.audit.clone();
                                drop(connection);
                                drop(flow);
                                abort_guest(&control, vec![guest]).await?;
                                audit.record(Type::NetworkConnectResult, reason, 0, 0).await?;
                                continue;
                            }
                            if flow._pending.lease.as_ref().is_none_or(|lease| lease.is_cancelled()) {
                                let guest = flow.guest;
                                let audit = flow.audit.clone();
                                drop(connection);
                                drop(flow);
                                abort_guest(&control, vec![guest]).await?;
                                audit.record(Type::NetworkConnectResult, NetworkReason::StaleGeneration, 0, 0).await?;
                                continue;
                            }
                            // Guest setup completes out of order. The child sees
                            // an independent, monotonic handoff sequence.
                            let grant = async {
                                let destination = connection.try_clone_fd()?;
                                capsem_foundation::unix::fd::set_stream_buffers(destination.as_fd(),
                                    capsem_foundation::unix::router_stream::SOCKET_BUFFER_SIZE)?;
                                flow.connection = Some(connection);
                                flow.acknowledgement = Some(Instant::now() + Duration::from_secs(2));
                                router
                                    .grant(
                                        flow.source.as_fd(),
                                        destination.as_fd(),
                                        queue.clone(),
                                        flow.preview,
                                    )
                                    .await
                            }.await;
                            match grant {
                                Ok(id) => { active.insert(id, flow); }
                                Err(error) => {
                                    let guest = flow.guest;
                                    drop(flow);
                                    abort_guest(&control, vec![guest]).await
                                        .with_context(|| format!("after router grant failed: {error:#}"))?;
                                    return Err(error);
                                }
                            }
                        }
                        Err(error) => {
                            let guest = flow.guest;
                            let audit = flow.audit.clone();
                            drop(flow);
                            tracing::debug!(connection_id = id, %error, "publication connection refused");
                            abort_guest(&control, vec![guest]).await?;
                            audit.record(Type::NetworkConnectResult, reason, 0, 0).await?;
                        }
                    }
                }
                _ = acknowledgements.tick() => {
                    // A preview flow lives no longer than the session that
                    // admitted it: expiry and revocation end open connections,
                    // not only admission (google/capsem#222).
                    let now = std::time::Instant::now();
                    let ended: Vec<(u64, NetworkReason)> = active
                        .iter()
                        .filter(|(_, flow)| flow.accepted && flow.ending.is_none())
                        .filter_map(|(&id, flow)| Some((id, flow.session.as_ref()?.ended(now)?)))
                        .collect();
                    for (id, reason) in ended {
                        {
                            let flow = active.get_mut(&id).context("ending an unknown connection")?;
                            flow.ending = Some(reason);
                            flow.close_deadline = Some(Instant::now() + Duration::from_secs(2));
                        }
                        router.cancel(id).await?;
                    }
                    ensure!(!active.values().any(|flow| flow.acknowledgement.is_some_and(|deadline| deadline <= Instant::now())),
                        "router acknowledgement timed out");
                    ensure!(!active.values().any(|flow| flow.close_deadline.is_some_and(|deadline| deadline <= Instant::now())),
                        "router close acknowledgement timed out");
                }
            }
        }
    }).await;
    if result.is_err() {
        // An invalid record or missing ACK means the child cannot be trusted to
        // relinquish its copies. Terminate the shared child, not just this broker.
        router.closed.cancel();
    }
    let ids: Vec<_> = active.keys().copied().collect();
    let guests = active
        .values()
        .chain(connecting.values())
        .map(|flow| flow.guest)
        .collect();
    let audits: Vec<_> = active
        .values()
        .chain(connecting.values())
        .map(|flow| {
            (
                flow.audit.clone(),
                if flow.accepted {
                    Type::NetworkClose
                } else {
                    Type::NetworkConnectResult
                },
            )
        })
        .collect();
    drop(active);
    drop(connecting);
    setups.shutdown().await;
    // Join setup first: no late ConnectPort may overtake this bounded batch.
    let guest_cleanup = abort_guest(&control, guests).await;
    let router_cleanup = tokio::time::timeout(Duration::from_secs(2), async {
        for id in ids {
            router.abort(id).await?;
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .context("router cleanup timed out")
    .and_then(|result| result);
    if router_cleanup.is_err() {
        router.closed.cancel();
    }
    let audit_cleanup = tokio::time::timeout(Duration::from_secs(2), async {
        for (audit, kind) in audits {
            audit.record(kind, NetworkReason::Cancelled, 0, 0).await?;
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .context("publication cleanup audit deadline exceeded")
    .and_then(|result| result);
    if let Err(error) = &audit_cleanup {
        tracing::error!(%error, "publication cleanup audit failed");
    }
    result.and(guest_cleanup).and(router_cleanup).and(audit_cleanup)
}

async fn abort_guest(
    control: &mpsc::Sender<ServiceToProcess>,
    flows: Vec<capsem_proto::router::FlowKey>,
) -> Result<()> {
    if flows.is_empty() {
        return Ok(());
    }
    ensure!(
        flows.len() <= capsem_proto::router::MAX_ABORT_FLOWS,
        "guest abort batch exceeds quota"
    );
    tokio::time::timeout(
        Duration::from_secs(2),
        control.send(ServiceToProcess::AbortPorts { flows }),
    )
    .await
    .context("guest abort admission timed out")?
    .context("guest abort control closed")
}
