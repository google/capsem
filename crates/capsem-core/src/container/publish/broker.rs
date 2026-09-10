use super::*;
use capsem_router::MAX_CONNECTIONS;
use tokio::net::TcpListener;
use tokio::time::Instant;

struct Active {
    _permit: tokio::sync::OwnedSemaphorePermit,
    setup: tokio::task::AbortHandle,
    source: std::net::TcpStream,
    connection: Option<VsockConnection>,
    acknowledgement: Option<Instant>,
    accepted: bool,
}
impl Drop for Active {
    fn drop(&mut self) {
        self.setup.abort();
        if let Err(error) = self.source.shutdown(std::net::Shutdown::Both) {
            tracing::debug!(%error, "publication source shutdown");
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
                    let source = source.into_std()?;
                    let (pending, receiver) = owner.request()?;
                    let id = pending.id;
                    let flow = capsem_proto::router::FlowKey { generation: owner.generation, id };
                    let control = control.clone();
                    let slots = owner.setups.clone();
                    let rate = owner.setup_rate.clone();
                    let setup = setups.spawn(async move {
                        let _pending = pending;
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
                    connecting.insert(id, Active { _permit: permit, setup, source, connection: None, acknowledgement: None, accepted: false });
                }
                event = records.recv() => match event.context("router event reader closed")? {
                    Event::Accepted(id) => {
                        let flow = active.get_mut(&id).context("router acknowledged unknown connection")?;
                        ensure!(flow.acknowledgement.take().is_some() && !flow.accepted, "invalid router acknowledgement");
                        flow.accepted = true;
                    }
                    Event::Closed(id, report) => {
                        let flow = active.remove(&id).context("router closed unknown connection")?;
                        ensure!(flow.accepted, "router closed unacknowledged connection");
                        tracing::debug!(connection_id = id, reason = ?report.reason,
                            from_source = report.from_source, to_source = report.to_source, "publication closed");
                        drop(flow);
                    }
                    Event::Refused(id) => {
                        let flow = active.remove(&id).context("router refused unknown connection")?;
                        ensure!(flow.acknowledgement.is_some() && !flow.accepted, "unexpected router refusal");
                        drop(flow);
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
                            let destination = connection.try_clone_fd()?;
                            capsem_foundation::unix::fd::set_stream_buffers(destination.as_fd(),
                                capsem_foundation::unix::router_stream::SOCKET_BUFFER_SIZE)?;
                            flow.connection = Some(connection);
                            flow.acknowledgement = Some(Instant::now() + Duration::from_secs(2));
                            let id = router.grant(flow.source.as_fd(), destination.as_fd(), queue.clone()).await?;
                            active.insert(id, flow);
                        }
                        Err(error) => {
                            drop(flow);
                            tracing::debug!(connection_id = id, %error, "publication connection refused");
                        }
                    }
                }
                _ = acknowledgements.tick() => {
                    ensure!(!active.values().any(|flow| flow.acknowledgement.is_some_and(|deadline| deadline <= Instant::now())),
                        "router acknowledgement timed out");
                }
            }
        }
    }.await;
    let ids: Vec<_> = active.keys().copied().collect();
    drop(active);
    drop(connecting);
    setups.shutdown().await;
    for id in ids {
        if let Err(error) = router.abort(id).await {
            tracing::debug!(%error, connection_id = id, "publication abort after cleanup");
        }
    }
    result
}
