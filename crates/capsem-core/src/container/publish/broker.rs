use super::*;
use capsem_router::MAX_CONNECTIONS;
use tokio::net::TcpListener;
use tokio::time::Instant;

struct Active {
    guest: capsem_proto::router::FlowKey,
    _pending: Pending,
    graceful: bool,
    _permit: tokio::sync::OwnedSemaphorePermit,
    setup: tokio::task::AbortHandle,
    source: Arc<std::net::TcpStream>,
    connection: Option<VsockConnection>,
    acknowledgement: Option<Instant>,
    accepted: bool,
    close_deadline: Option<Instant>,
}
impl Drop for Active {
    fn drop(&mut self) {
        self.setup.abort();
        let shutdown = if self.graceful {
            capsem_foundation::unix::fd::tcp_clear_reset_on_close(self.source.as_fd())
                .and_then(|_| self.source.shutdown(std::net::Shutdown::Both))
        } else {
            capsem_foundation::unix::fd::reset_tcp(self.source.as_fd()).map(|_| ())
        };
        if let Err(error) = shutdown {
            tracing::debug!(%error, "publication source shutdown");
            if let Err(error) = self.source.shutdown(std::net::Shutdown::Both) {
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
    guest_port: u16,
    listener: TcpListener,
    control: mpsc::Sender<ServiceToProcess>,
    router: Arc<companion::Router>,
    cancellation: CancellationToken,
) -> Result<()> {
    let mut active: HashMap<u64, Active> = HashMap::new();
    let mut connecting: HashMap<u64, Active> = HashMap::new();
    let mut setups = tokio::task::JoinSet::new();
    let (queue, mut records) = mpsc::channel(MAX_CONNECTIONS);
    let (guest_close, mut guest_records) = mpsc::channel(MAX_CONNECTIONS);
    let mut acknowledgements = tokio::time::interval(Duration::from_millis(100));
    let result = async {
        loop {
            tokio::select! {
                _ = cancellation.cancelled() => return Ok(()),
                _ = router.closed.cancelled() => anyhow::bail!("VM router closed"),
                accepted = listener.accept(), if active.len() + connecting.len() < MAX_CONNECTIONS => {
                    let (source, peer) = accepted?;
                    let Ok(permit) = owner.ingress.clone().try_acquire_owned() else {
                        tracing::debug!(%peer, "VM ingress connection quota exhausted");
                        continue;
                    };
                    source.set_nodelay(true)?;
                    capsem_foundation::unix::fd::set_stream_buffers(source.as_fd(),
                        capsem_foundation::unix::router_stream::SOCKET_BUFFER_SIZE)?;
                    capsem_foundation::unix::fd::tcp_reset_on_close(source.as_fd())?;
                    let source = Arc::new(source.into_std()?);
                    let (pending, receiver) = owner.request(&source, guest_close.clone())?;
                    let id = pending.id;
                    let flow = capsem_proto::router::FlowKey { generation: owner.generation, id };
                    let control = control.clone();
                    let slots = owner.setups.clone();
                    let rate = owner.setup_rate.clone();
                    let setup = setups.spawn(async move {
                        let result = tokio::time::timeout(Duration::from_secs(8), async {
                            let _setup = slots.acquire_owned().await.context("guest setup admission closed")?;
                            rate.acquire().await;
                            control.send(ServiceToProcess::ConnectPort { flow, port: guest_port }).await
                                .context("guest control closed")?;
                            receiver.await.context("guest connection cancelled")?
                        }).await.context("guest connection timed out").and_then(|result| result);
                        (id, result)
                    });
                    tracing::debug!(connection_id = id, %peer, guest_port, "publication connection accepted");
                    connecting.insert(id, Active { guest: flow, _pending: pending, graceful: false, _permit: permit, setup, source, connection: None, acknowledgement: None, accepted: false, close_deadline: None });
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
                        let flow = active.get_mut(&id).context("router acknowledged unknown connection")?;
                        ensure!(flow.acknowledgement.take().is_some() && !flow.accepted, "invalid router acknowledgement");
                        flow.accepted = true;
                    }
                    Event::Closed(id, report) => {
                        ensure!(active.get(&id).is_some_and(|flow| flow.accepted), "router closed unacknowledged connection");
                        let mut flow = active.remove(&id).unwrap();
                        flow.graceful = report.reason == capsem_proto::router::CloseReason::Complete;
                        tracing::debug!(connection_id = id, reason = ?report.reason,
                            from_source = report.from_source, to_source = report.to_source, "publication closed");
                        let guest = flow.guest;
                        drop(flow);
                        if report.reason != capsem_proto::router::CloseReason::Complete {
                            abort_guest(&control, vec![guest]).await?;
                        }
                    }
                    Event::Refused(id) => {
                        ensure!(active.get(&id).is_some_and(|flow| flow.acknowledgement.is_some() && !flow.accepted), "unexpected router refusal");
                        let flow = active.remove(&id).unwrap();
                        let guest = flow.guest;
                        drop(flow);
                        abort_guest(&control, vec![guest]).await?;
                    }
                    Event::Ready | Event::ConfinementFailed => anyhow::bail!("unexpected router startup event"),
                },
                completed = setups.join_next(), if !setups.is_empty() => {
                    let (id, result) = completed.unwrap().context("publication setup task failed")?;
                    let mut flow = connecting.remove(&id).context("completed unknown publication setup")?;
                    match result {
                        Ok(connection) => {
                            // Guest setup completes out of order. The child sees
                            // an independent, monotonic handoff sequence.
                            let grant = async {
                                let destination = connection.try_clone_fd()?;
                                capsem_foundation::unix::fd::set_stream_buffers(destination.as_fd(),
                                    capsem_foundation::unix::router_stream::SOCKET_BUFFER_SIZE)?;
                                flow.connection = Some(connection);
                                flow.acknowledgement = Some(Instant::now() + Duration::from_secs(2));
                                router.grant(flow.source.as_fd(), destination.as_fd(), queue.clone()).await
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
                            drop(flow);
                            tracing::debug!(connection_id = id, %error, "publication connection refused");
                            abort_guest(&control, vec![guest]).await?;
                        }
                    }
                }
                _ = acknowledgements.tick() => {
                    ensure!(!active.values().any(|flow| flow.acknowledgement.is_some_and(|deadline| deadline <= Instant::now())),
                        "router acknowledgement timed out");
                    ensure!(!active.values().any(|flow| flow.close_deadline.is_some_and(|deadline| deadline <= Instant::now())),
                        "router close acknowledgement timed out");
                }
            }
        }
    }.await;
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
    result.and(guest_cleanup).and(router_cleanup)
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
