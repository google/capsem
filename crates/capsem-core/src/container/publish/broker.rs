use super::*;
use capsem_router::MAX_CONNECTIONS;
use tokio::net::TcpListener;
use tokio::time::Instant;

struct Active {
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
    sender: capsem_foundation::unix::router_channel::Sender,
    mut events: UnixStream,
    cancellation: CancellationToken,
) -> Result<()> {
    let mut active: HashMap<u64, Active> = HashMap::new();
    let mut connecting: HashMap<u64, Active> = HashMap::new();
    let mut next_grant: u64 = 1;
    let mut setups = tokio::task::JoinSet::new();
    let mut readers = tokio::task::JoinSet::new();
    let (queue, mut records) = mpsc::channel(16);
    readers.spawn(async move {
        loop {
            let event = Event::read(&mut events).await;
            let failed = event.is_err();
            if queue.send(event).await.is_err() || failed {
                break;
            }
        }
    });
    let mut acknowledgements = tokio::time::interval(Duration::from_millis(100));
    let result = async {
        loop {
            tokio::select! {
                _ = cancellation.cancelled() => return Ok(()),
                accepted = listener.accept(), if active.len() + connecting.len() < MAX_CONNECTIONS => {
                    let (source, peer) = accepted?;
                    source.set_nodelay(true)?;
                    let source = source.into_std()?;
                    let (pending, receiver) = owner.request()?;
                    let id = pending.id;
                    let control = control.clone();
                    let setup = setups.spawn(async move {
                        let _pending = pending;
                        let result = tokio::time::timeout(Duration::from_secs(8), async {
                            control.send(ServiceToProcess::ConnectPort { id, port: guest_port }).await
                                .context("guest control closed")?;
                            receiver.await.context("guest connection cancelled")?
                        }).await.context("guest connection timed out").and_then(|result| result);
                        (id, result)
                    });
                    tracing::debug!(connection_id = id, %peer, guest_port, "publication connection accepted");
                    connecting.insert(id, Active { setup, source, connection: None, acknowledgement: None, accepted: false });
                }
                event = records.recv() => match event.context("router event reader closed")?? {
                    Event::Accepted(id) => {
                        let flow = active.get_mut(&id).context("router acknowledged unknown connection")?;
                        ensure!(flow.acknowledgement.take().is_some() && !flow.accepted, "invalid router acknowledgement");
                        flow.accepted = true;
                    }
                    Event::Closed(id) => {
                        let flow = active.remove(&id).context("router closed unknown connection")?;
                        ensure!(flow.accepted, "router closed unacknowledged connection");
                    }
                    Event::Refused(id) => {
                        let flow = active.remove(&id).context("router refused unknown connection")?;
                        ensure!(flow.acknowledgement.is_some() && !flow.accepted, "unexpected router refusal");
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
                            let id = next_grant;
                            next_grant = next_grant.checked_add(1).context("router grant ids exhausted")?;
                            let destination = connection.try_clone_fd()?;
                            flow.connection = Some(connection);
                            flow.acknowledgement = Some(Instant::now() + Duration::from_secs(2));
                            tokio::time::timeout(Duration::from_secs(2), send_grant(&sender, Grant::Connected {
                                id, class: capsem_router::Class::Expose, source: flow.source.as_fd(), destination: destination.as_fd()
                            })).await??;
                            active.insert(id, flow);
                        }
                        Err(error) => {
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
    active.clear();
    connecting.clear();
    setups.shutdown().await;
    readers.shutdown().await;
    result
}
