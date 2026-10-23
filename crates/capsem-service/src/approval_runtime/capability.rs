//! The existing coordinator grants a connected, generation-bound descriptor.
use super::*;
use capsem_foundation::ipc_channel;
use capsem_proto::approval::{ApprovalClientMessage, ApprovalServerMessage};
use capsem_proto::ledger::LedgerClientRole;
use std::os::unix::net::UnixStream;

pub(crate) type Server = std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send>>;

impl Runtime {
    pub(crate) async fn open(
        &self,
        state: &Arc<crate::ServiceState>,
        binding: ApprovalBinding,
        directory: &std::path::Path,
        authority: WorkerGrant,
        policy: tokio::sync::watch::Receiver<Arc<crate::upstream_broker::BrokerPolicy>>,
    ) -> Result<(UnixStream, Server), String> {
        // Serialize only finite channel setup, never a human decision.
        let initialization = self.initialization.lock().await;
        let session = if let Some(session) = self.get(&binding) {
            session
        } else {
            let path = directory.join("session.db");
            let (stream, grant) = state
                .ledger_workers
                .acquire(
                    &binding.session_id,
                    &path,
                    &directory.join("ledger.log"),
                    LedgerClientRole::Coordinator,
                )
                .await
                .map_err(|error| format!("approval ledger capability: {error:#}"))?
                .into_parts();
            let writer = tokio::task::spawn_blocking(move || {
                DbWriter::from_ledger_channel(stream, grant, &path, capsem_proto::APPROVAL_PENDING_PER_SESSION)
            })
            .await
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string())?;
            let reader = state
                .register_session_db_handle_async(&binding.session_id, directory)
                .await
                .map_err(|error| format!("approval ledger reader: {error:#}"))?;
            self.install(binding, Arc::new(writer), reader, authority)?
        };
        drop(initialization);
        let (service, worker) = UnixStream::pair().map_err(|error| error.to_string())?;
        // The broker supervises this task and joins its terminal audit before
        // allowing ledger lifecycle teardown.
        Ok((worker, Box::pin(server(service, session, policy))))
    }
}

struct Requests {
    session: Arc<Session>,
    ids: HashMap<u64, String>,
}
impl Drop for Requests {
    fn drop(&mut self) {
        for ask in self.ids.values() {
            self.session.broker.cancel(&self.session.binding, ask);
        }
    }
}

pub(super) async fn server(
    stream: UnixStream,
    session: Arc<Session>,
    mut policy: tokio::sync::watch::Receiver<Arc<crate::upstream_broker::BrokerPolicy>>,
) -> Result<(), String> {
    let (sender, receiver) = ipc_channel::channel_from_std_with_limit::<ApprovalServerMessage, ApprovalClientMessage>(
        stream,
        capsem_core::security_engine::approval::client::frame_limit(),
    )
    .map_err(|error| error.to_string())?;
    let sender = Arc::new(sender);
    let mut requests = Requests {
        session: Arc::clone(&session),
        ids: HashMap::new(),
    };
    let mut tasks = tokio::task::JoinSet::new();
    let mut highest = 0;
    loop {
        tokio::select! {
            biased;
            _ = session.authority.revoked() => break,
            changed = policy.changed() => {
                for ask in requests.ids.values() { session.broker.cancel(&session.binding, ask); }
                if changed.is_err() { break; }
            }
            completed = tasks.join_next(), if !tasks.is_empty() => {
                let Some(Ok((id, outcome))) = completed else { break; };
                requests.ids.remove(&id);
                let sent = tokio::time::timeout(std::time::Duration::from_secs(5), sender.send(ApprovalServerMessage::Resolved { request_id: id, outcome })).await;
                if !matches!(sent, Ok(Ok(()))) { break; }
            }
            message = receiver.recv() => match message {
                Ok(ApprovalClientMessage::Begin { request_id, request }) => {
                    if request_id <= highest { break; }
                    highest = request_id;
                    let pending = if tasks.len() < capsem_proto::APPROVAL_PENDING_PER_SESSION { session.reserve(&request).ok() } else { None };
                    if let Some(pending) = pending {
                        requests.ids.insert(request_id, request.ask_id.clone());
                        let session = Arc::clone(&session);
                        let registered = Arc::clone(&sender);
                        tasks.spawn(async move {
                            let outcome = if session.verify(&request).await.is_ok() {
                                if let Some(registration) = pending.publish_registered() {
                                    let sent = tokio::time::timeout(std::time::Duration::from_secs(5), registered.send(
                                        ApprovalServerMessage::Registered {request_id,registration}
                                    )).await;
                                    if !matches!(sent,Ok(Ok(()))) { session.broker.cancel(&session.binding,&request.ask_id); }
                                }
                                session.wait(&request.ask_id, pending).await
                            } else { ApprovalOutcome::Unavailable };
                            (request_id, outcome)
                        });
                    } else {
                        let sent = tokio::time::timeout(std::time::Duration::from_secs(5), sender.send(ApprovalServerMessage::Resolved { request_id, outcome: ApprovalOutcome::Unavailable })).await;
                        if !matches!(sent, Ok(Ok(()))) { break; }
                    }
                }
                Ok(ApprovalClientMessage::Cancel { request_id }) => {
                    if let Some(ask) = requests.ids.get(&request_id) { session.broker.cancel(&session.binding, ask); }
                }
                Err(_) => break,
            }
        }
    }
    for ask in requests.ids.values() {
        session.broker.cancel(&session.binding, ask);
    }
    while tasks.join_next().await.is_some() {}
    Ok(())
}
