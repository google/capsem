//! Request-independent enrollment jobs with durable request IDs and finite ownership.
use super::*;

impl ConnectionBroker {
    pub async fn authorize(&self, request: api::AuthorizeGoogleRequest) -> BrokerResult<api::AuthorizeGoogleResponse> {
        uuid(&request.request_id)?;
        let mut state = self.0.state.lock().await;
        if let Some(entry) = state
            .transactions
            .values()
            .find(|entry| entry.record.request.request_id == request.request_id)
        {
            if entry.record.request != request {
                return Err(api::CredentialErrorCode::RequestConflict);
            }
            return Ok(response(entry));
        }
        let profile = self
            .0
            .profiles
            .get(&request.scope_profile_id)
            .ok_or(api::CredentialErrorCode::UnknownScopeProfile)?;
        let provider = profile
            .provider
            .as_ref()
            .filter(|_| profile.summary.enabled)
            .ok_or(api::CredentialErrorCode::ConfigurationRequired)?
            .clone();
        if state.transactions.len() >= self.0.limits.max_transactions {
            return Err(api::CredentialErrorCode::StorageUnavailable);
        }
        let reconnect = match &request.reconnect_connection_id {
            Some(id) => {
                let entry = state
                    .connections
                    .get(id)
                    .ok_or(api::CredentialErrorCode::ConnectionUnavailable)?;
                if entry.record.scope_profile_id != request.scope_profile_id
                    || entry.record.retiring
                    || state.transactions.values().any(|entry| {
                        entry.record.target_connection_id == *id
                            && entry.record.transaction.state == api::TransactionState::Pending
                    })
                {
                    return Err(api::CredentialErrorCode::RequestConflict);
                }
                Some(
                    entry
                        .connection
                        .clone()
                        .ok_or(api::CredentialErrorCode::ConnectionUnavailable)?,
                )
            }
            None => {
                let pending = state
                    .transactions
                    .values()
                    .filter(|entry| {
                        entry.record.transaction.state == api::TransactionState::Pending
                            && entry.record.request.reconnect_connection_id.is_none()
                    })
                    .count();
                if state.connections.len() + pending >= self.0.limits.max_connections {
                    return Err(api::CredentialErrorCode::StorageUnavailable);
                }
                None
            }
        };
        let target = request
            .reconnect_connection_id
            .clone()
            .unwrap_or_else(|| capsem_crypto::uuid_v4().to_string());
        let id = capsem_crypto::uuid_v4().to_string();
        let deadline = Instant::now() + self.0.limits.enrollment_lifetime;
        let record = TransactionRecord {
            request,
            terminal_at: None,
            target_connection_id: target.clone(),
            transaction: api::AuthorizationTransaction {
                id: id.clone(),
                state: api::TransactionState::Pending,
                expires_at: timestamp(SystemTime::now() + self.0.limits.enrollment_lifetime)?,
                connection_id: None,
                error_code: None,
            },
        };
        state
            .transactions
            .insert(id.clone(), TransactionEntry { record, url: None });
        if let Err(error) = self.0.store.write(state.snapshot()).await {
            fail(&mut state, &id, error);
            return Err(error);
        }
        let (sent, received) = tokio::sync::oneshot::channel();
        let owner = Arc::downgrade(&self.0);
        let work_id = id.clone();
        let job = tokio::spawn(async move {
            run(owner, work_id, provider, target, reconnect, deadline, sent).await;
        });
        state.jobs.insert(id, job);
        drop(state);
        received
            .await
            .map_err(|_| api::CredentialErrorCode::TransactionCancelled)?
    }

    pub async fn authorization(&self, id: &str) -> BrokerResult<api::AuthorizationTransaction> {
        let state = self.0.state.lock().await;
        Ok(state
            .transactions
            .get(id)
            .ok_or(api::CredentialErrorCode::TransactionExpired)?
            .record
            .transaction
            .clone())
    }

    pub async fn cancel_authorization(&self, id: &str) -> BrokerResult<api::AuthorizationTransaction> {
        let mut state = self.0.state.lock().await;
        let entry = state
            .transactions
            .get_mut(id)
            .ok_or(api::CredentialErrorCode::TransactionExpired)?;
        if entry.record.transaction.state != api::TransactionState::Pending {
            return Ok(entry.record.transaction.clone());
        }
        entry.record.transaction.state = api::TransactionState::Cancelled;
        entry.record.terminal_at = Some(SystemTime::now());
        entry.record.transaction.error_code = Some(api::CredentialErrorCode::TransactionCancelled);
        entry.url = None;
        let result = entry.record.transaction.clone();
        let job = state.jobs.remove(id);
        if let Some(job) = &job {
            job.abort();
        }
        let saved = self.0.store.write(state.snapshot()).await;
        drop(state);
        if let Some(job) = job {
            let _ = job.await;
        }
        saved?;
        Ok(result)
    }

    pub async fn disconnect(&self, id: &str) -> BrokerResult<api::DisconnectResponse> {
        let mut state = self.0.state.lock().await;
        let entry = state
            .connections
            .get_mut(id)
            .ok_or(api::CredentialErrorCode::ConnectionUnavailable)?;
        entry
            .connection
            .as_ref()
            .ok_or(api::CredentialErrorCode::ConnectionUnavailable)?;
        entry.record.retiring = true;
        let count = self.retire_connection_grants(&mut state, id)?;
        let saved = self.0.store.write(state.snapshot()).await;
        let response = api::DisconnectResponse {
            connection: self.summary(state.connections.get(id).expect("retained connection"))?,
            revoked_grant_count: count,
            local_authority: api::LocalCredentialAuthority::Revoked,
        };
        self.start_retirement(&mut state, id)?;
        drop(state);
        saved?;
        Ok(response)
    }
}

fn response(entry: &TransactionEntry) -> api::AuthorizeGoogleResponse {
    api::AuthorizeGoogleResponse {
        transaction: entry.record.transaction.clone(),
        authorization_url: entry
            .url
            .as_ref()
            .filter(|_| entry.record.transaction.state == api::TransactionState::Pending)
            .map(|url| url.to_string()),
    }
}
fn fail(state: &mut State, id: &str, code: api::CredentialErrorCode) {
    if let Some(entry) = state.transactions.get_mut(id) {
        if entry.record.transaction.state != api::TransactionState::Pending {
            return;
        }
        entry.record.transaction.state = if code == api::CredentialErrorCode::TransactionExpired {
            api::TransactionState::Expired
        } else {
            api::TransactionState::Failed
        };
        entry.record.transaction.error_code = Some(code);
        entry.record.terminal_at = Some(SystemTime::now());
        entry.url = None;
    }
}

async fn run(
    owner: Weak<Inner>,
    id: String,
    provider: Arc<dyn EnrollmentProvider>,
    target: String,
    reconnect: Option<GoogleConnection>,
    deadline: Instant,
    sent: tokio::sync::oneshot::Sender<BrokerResult<api::AuthorizeGoogleResponse>>,
) {
    let begun = tokio::time::timeout_at(
        tokio::time::Instant::from_std(deadline),
        provider.begin(target.clone(), reconnect.clone()),
    )
    .await
    .map_err(|_| api::CredentialErrorCode::TransactionExpired)
    .and_then(|result| result);
    let attempt = match begun {
        Ok(attempt) => attempt,
        Err(error) => {
            if let Some(owner) = owner.upgrade() {
                let mut state = owner.state.lock().await;
                fail(&mut state, &id, error);
                let _ = owner.store.write(state.snapshot()).await;
                state.jobs.remove(&id);
            }
            let _ = sent.send(Err(error));
            return;
        }
    };
    {
        let Some(inner) = owner.upgrade() else { return };
        let mut state = inner.state.lock().await;
        let Some(entry) = state.transactions.get_mut(&id) else {
            return;
        };
        if entry.record.transaction.state != api::TransactionState::Pending {
            return;
        }
        entry.url = attempt.authorization_url;
        let result = response(entry);
        drop(state);
        let _ = sent.send(Ok(result));
    }
    let completed = tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), attempt.completion)
        .await
        .map_err(|_| api::CredentialErrorCode::TransactionExpired)
        .and_then(|result| result);
    let Some(inner) = owner.upgrade() else { return };
    let broker = ConnectionBroker(inner);
    let mut state = broker.0.state.lock().await;
    match completed {
        Ok(connection) => {
            let pending = state
                .transactions
                .get(&id)
                .is_some_and(|entry| entry.record.transaction.state == api::TransactionState::Pending);
            if !pending
                || reconnect
                    .as_ref()
                    .is_some_and(|original| !original.same_owner(&connection))
                || state
                    .connections
                    .get(&target)
                    .is_some_and(|entry| entry.record.retiring)
                || connection.authorization().is_err()
            {
                fail(&mut state, &id, api::CredentialErrorCode::RequestConflict);
                drop(state);
                let _ = connection.disconnect().await;
                return;
            }
            if reconnect.is_some() {
                let _ = broker.retire_connection_grants(&mut state, &target);
            }
            let profile = state
                .transactions
                .get(&id)
                .expect("pending transaction")
                .record
                .request
                .scope_profile_id
                .clone();
            state
                .connections
                .entry(target.clone())
                .and_modify(|entry| {
                    entry.connection = Some(connection.clone());
                    entry.record.retiring = false;
                    entry.record.error_code = None;
                })
                .or_insert(ConnectionEntry {
                    record: ConnectionRecord {
                        id: target.clone(),
                        scope_profile_id: profile,
                        retiring: false,
                        created_at: SystemTime::now(),
                        error_code: None,
                    },
                    connection: Some(connection.clone()),
                });
            let entry = state.transactions.get_mut(&id).expect("pending transaction");
            entry.record.transaction.state = api::TransactionState::Connected;
            entry.record.terminal_at = Some(SystemTime::now());
            entry.record.transaction.connection_id = Some(target.clone());
            entry.url = None;
            if broker.0.store.write(state.snapshot()).await.is_err() {
                let entry = state.connections.get_mut(&target).expect("new connection");
                entry.record.retiring = true;
                entry.record.error_code = Some(api::CredentialErrorCode::StorageCommitFailed);
                let entry = state.transactions.get_mut(&id).expect("pending transaction");
                entry.record.transaction.state = api::TransactionState::Failed;
                entry.record.transaction.error_code = Some(api::CredentialErrorCode::StorageCommitFailed);
                state.jobs.remove(&id);
                drop(state);
                let _ = connection.disconnect().await;
                return;
            }
        }
        Err(error) => {
            fail(&mut state, &id, error);
            let _ = broker.0.store.write(state.snapshot()).await;
        }
    }
    state.jobs.remove(&id);
}
