use super::*;
use capsem_port_router::MAX_CONNECTIONS;

struct Active {
    setup: tokio::task::AbortHandle,
    _connection: Option<VsockConnection>,
}
impl Drop for Active {
    fn drop(&mut self) {
        self.setup.abort();
    }
}

pub(super) async fn serve(
    owner: Arc<Publisher>,
    guest_port: u16,
    control: mpsc::Sender<ServiceToProcess>,
    sender: capsem_foundation::unix::router_channel::Sender,
    mut events: UnixStream,
) -> Result<()> {
    let mut active: HashMap<u64, Active> = HashMap::new();
    let mut setups = tokio::task::JoinSet::new();
    // Frame readers run to completion. Cancelling read_exact midway through a
    // record when another select branch wins would lose framing state.
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
    let mut last_id = 0;
    loop {
        tokio::select! {
            event = records.recv() => match event.context("router event reader closed")?? {
                Event::Open(id) => {
                    ensure!(id > last_id && active.len() < MAX_CONNECTIONS, "router exceeded its connection authority");
                    last_id = id;
                    let (pending, receiver) = owner.request()?;
                    let global_id = pending.id;
                    let setup = setups.spawn(async move {
                        let _pending = pending;
                        let result = async {
                            tokio::time::timeout(Duration::from_secs(8), receiver).await.context("guest connection timed out")?
                                .context("guest connection cancelled")?
                        }.await;
                        (id, result)
                    });
                    active.insert(id, Active { setup, _connection: None });
                    tokio::time::timeout(Duration::from_secs(1), control.send(ServiceToProcess::ConnectPort { id: global_id, port: guest_port })).await??;
                }
                Event::Closed(id) => {
                    ensure!(active.remove(&id).is_some(), "router closed an unknown connection");
                }
                Event::Ready => anyhow::bail!("duplicate router ready event"),
            },
            completed = setups.join_next(), if !setups.is_empty() => {
                let (id, result) = match completed.unwrap() {
                    Ok(result) => result,
                    Err(error) if error.is_cancelled() => continue,
                    Err(error) => return Err(error.into()),
                };
                let Some(active) = active.get_mut(&id) else { continue; };
                match result {
                    Ok(connection) => {
                        let fd = connection.try_clone_fd()?;
                        tokio::time::timeout(Duration::from_secs(2), send_grant(&sender, Grant::Connected { id, socket: fd.as_fd() })).await??;
                        active._connection = Some(connection);
                    }
                    Err(error) => {
                        tracing::debug!(id, %error, "publication connection refused");
                        tokio::time::timeout(Duration::from_secs(2), send_grant(&sender, Grant::Refused { id })).await??;
                    }
                }
            }
        }
    }
}
