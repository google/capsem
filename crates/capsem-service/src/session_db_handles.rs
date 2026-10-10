//! Authenticated per-session ledger channels held by service routes.

use super::*;
use capsem_logger::ledger_protocol::{LedgerQuery, LedgerRows};
use capsem_proto::ledger::LedgerClientRole;
use capsem_proto::ledger_counters::LedgerCounters;

pub(super) fn session_db_path_for_session_dir(session_dir: &StdPath) -> PathBuf {
    session_dir.join("session.db")
}

pub(crate) struct SessionLedger {
    path: PathBuf,
    remote: Option<RemoteLedger>,
    #[cfg(test)]
    embedded: Option<capsem_logger::DbHandle>,
}

struct RemoteLedger {
    vm_id: String,
    session_dir: PathBuf,
    workers: Arc<crate::ledger_worker::LedgerWorkers>,
    client: tokio::sync::Mutex<capsem_logger::ledger_client::LedgerClient>,
    observed_remote_epoch: Mutex<Option<u64>>,
    read_cache_epoch: AtomicU64,
}

impl SessionLedger {
    #[cfg(not(test))]
    fn remote(
        vm_id: String,
        session_dir: PathBuf,
        workers: Arc<crate::ledger_worker::LedgerWorkers>,
        client: capsem_logger::ledger_client::LedgerClient,
    ) -> Self {
        Self {
            path: session_db_path_for_session_dir(&session_dir),
            remote: Some(RemoteLedger {
                vm_id,
                session_dir,
                workers,
                client: tokio::sync::Mutex::new(client),
                observed_remote_epoch: Mutex::new(None),
                read_cache_epoch: AtomicU64::new(0),
            }),
            #[cfg(test)]
            embedded: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn embedded(path: PathBuf, db: capsem_logger::DbHandle) -> Self {
        Self {
            path,
            remote: None,
            embedded: Some(db),
        }
    }

    pub(crate) fn path(&self) -> &StdPath {
        &self.path
    }

    pub(crate) async fn ready(&self) -> Result<(), String> {
        #[cfg(test)]
        if let Some(db) = &self.embedded {
            return db.ready().await;
        }
        self.remote_state().refresh_counters().await.map(|_| ())
    }

    pub(crate) async fn query(&self, query: LedgerQuery) -> Result<Vec<LedgerRows>, String> {
        #[cfg(test)]
        if let Some(db) = &self.embedded {
            return capsem_logger::ledger_server::execute_named_query(db, query).await;
        }
        self.remote_state()
            .call(|client| {
                let query = query.clone();
                async move { client.query(query).await }
            })
            .await
    }

    pub(crate) async fn ledger_counters(&self) -> Result<Arc<LedgerCounters>, String> {
        #[cfg(test)]
        if let Some(db) = &self.embedded {
            return db.ledger_counters().await;
        }
        self.remote_state().refresh_counters().await
    }

    pub(crate) fn read_cache_epoch(&self, _domain: capsem_logger::ReadCacheDomain) -> u64 {
        #[cfg(test)]
        if let Some(db) = &self.embedded {
            return db.read_cache_epoch(capsem_logger::ReadCacheDomain::All);
        }
        self.remote_state().read_cache_epoch.load(Ordering::Acquire)
    }

    #[cfg(test)]
    pub(crate) fn reader_requests(&self) -> u64 {
        self.embedded
            .as_ref()
            .map_or(0, capsem_logger::DbHandle::reader_requests)
    }

    pub(crate) async fn read_bodies(&self, event_id: &str) -> Result<Vec<capsem_logger::StoredBody>, String> {
        #[cfg(test)]
        if let Some(db) = &self.embedded {
            return db.read_bodies(event_id).await;
        }
        let event_id = event_id.to_string();
        self.remote_state()
            .call(|client| {
                let event_id = event_id.clone();
                async move { client.read_bodies(&event_id).await }
            })
            .await
    }

    pub(crate) async fn export_warc(&self) -> Result<SessionWarcExport, String> {
        #[cfg(test)]
        if let Some(db) = &self.embedded {
            let bytes = Arc::new(Mutex::new(Vec::new()));
            let writer = TestWarcWriter(Arc::clone(&bytes));
            let summary = db.export_warc(writer).await?;
            let bytes = std::mem::take(&mut *bytes.lock().unwrap());
            return Ok(SessionWarcExport::Embedded {
                bytes: Some(bytes),
                summary: capsem_logger::ledger_protocol::LedgerExportSummary {
                    records: summary.records,
                    bytes_written: summary.bytes_written,
                    skipped_count: summary.skipped_count,
                    skipped_by_reason: summary
                        .counts_by_reason()
                        .into_iter()
                        .map(|(reason, count)| (reason.to_string(), count))
                        .collect(),
                },
            });
        }
        Ok(SessionWarcExport::Remote(
            self.remote_state()
                .call(|client| async move { client.export_warc().await })
                .await?,
        ))
    }

    fn remote_state(&self) -> &RemoteLedger {
        self.remote
            .as_ref()
            .expect("production session ledger has a remote client")
    }
}

impl RemoteLedger {
    async fn refresh_counters(&self) -> Result<Arc<LedgerCounters>, String> {
        let (current, remote_epoch) = self
            .call(|client| async move {
                let counters = client.counters().await?;
                Ok((counters, client.read_cache_epoch()))
            })
            .await?;
        let mut observed = self.observed_remote_epoch.lock().unwrap();
        if *observed != Some(remote_epoch) {
            *observed = Some(remote_epoch);
            self.read_cache_epoch.fetch_add(1, Ordering::AcqRel);
        }
        drop(observed);
        Ok(current)
    }

    async fn call<T, F, Fut>(&self, operation: F) -> Result<T, String>
    where
        F: Fn(capsem_logger::ledger_client::LedgerClient) -> Fut,
        Fut: std::future::Future<Output = Result<T, String>>,
    {
        let mut client = self.client.lock().await;
        let first = operation(client.clone()).await;
        let Err(first_error) = first else {
            drop(client);
            return first;
        };
        let replacement = self
            .reconnect()
            .await
            .map_err(|error| format!("ledger operation failed ({first_error}); reconnect failed: {error}"))?;
        *self.observed_remote_epoch.lock().unwrap() = None;
        *client = replacement.clone();
        drop(client);
        operation(replacement).await
    }

    async fn reconnect(&self) -> Result<capsem_logger::ledger_client::LedgerClient, String> {
        let database = session_db_path_for_session_dir(&self.session_dir);
        let log = self.session_dir.join("ledger.log");
        let mut last_error = String::new();
        for _ in 0..2 {
            match self
                .workers
                .acquire(&self.vm_id, &database, &log, LedgerClientRole::Reader)
                .await
            {
                Ok(channel) => {
                    let (stream, commitment, grant) = channel.into_parts();
                    debug_assert!(commitment.is_none());
                    return capsem_logger::ledger_client::LedgerClient::connect(stream, grant, database).await;
                }
                Err(error) => {
                    last_error = format!("{error:#}");
                    tokio::task::yield_now().await;
                }
            }
        }
        Err(last_error)
    }
}

#[cfg(test)]
struct TestWarcWriter(Arc<Mutex<Vec<u8>>>);

#[cfg(test)]
impl std::io::Write for TestWarcWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub(crate) enum SessionWarcExport {
    Remote(capsem_logger::ledger_client::LedgerWarcExport),
    #[cfg(test)]
    Embedded {
        bytes: Option<Vec<u8>>,
        summary: capsem_logger::ledger_protocol::LedgerExportSummary,
    },
}

impl SessionWarcExport {
    pub(crate) async fn next_chunk(&mut self) -> Option<Result<Vec<u8>, String>> {
        match self {
            Self::Remote(export) => export.next_chunk().await,
            #[cfg(test)]
            Self::Embedded { bytes, .. } => bytes.take().map(Ok),
        }
    }

    pub(crate) async fn finish(self) -> Result<capsem_logger::ledger_protocol::LedgerExportSummary, String> {
        match self {
            Self::Remote(export) => export.finish().await,
            #[cfg(test)]
            Self::Embedded { summary, .. } => Ok(summary),
        }
    }
}

impl ServiceState {
    #[cfg(test)]
    pub(crate) fn register_session_db_handle(
        &self,
        vm_id: &str,
        session_dir: &StdPath,
    ) -> anyhow::Result<Arc<SessionLedger>> {
        let db_path = session_db_path_for_session_dir(session_dir);
        let started = std::time::Instant::now();
        if let Some(handle) = self.registered_session_db_handle(vm_id, &db_path, started) {
            return Ok(handle);
        }
        let opened = crate::tests::test_session_ledger(db_path.clone());
        self.install_session_db_handle(vm_id, db_path, opened, started)
    }

    pub(crate) async fn register_session_db_handle_async(
        &self,
        vm_id: &str,
        session_dir: &StdPath,
    ) -> anyhow::Result<Arc<SessionLedger>> {
        #[cfg(test)]
        return self.register_session_db_handle(vm_id, session_dir);

        #[cfg(not(test))]
        let db_path = session_db_path_for_session_dir(session_dir);
        #[cfg(not(test))]
        let started = std::time::Instant::now();
        #[cfg(not(test))]
        if let Some(handle) = self.registered_session_db_handle(vm_id, &db_path, started) {
            return Ok(handle);
        }
        #[cfg(not(test))]
        let opened = self
            .ledger_workers
            .acquire(
                vm_id,
                &db_path,
                &session_dir.join("ledger.log"),
                LedgerClientRole::Reader,
            )
            .await
            .map_err(|error| error.to_string())
            .map(|channel| {
                let (stream, commitment, grant) = channel.into_parts();
                debug_assert!(commitment.is_none());
                (stream, grant)
            });
        #[cfg(not(test))]
        let opened = match opened {
            Ok((stream, grant)) => capsem_logger::ledger_client::LedgerClient::connect(stream, grant, db_path.clone())
                .await
                .map(|client| {
                    SessionLedger::remote(
                        vm_id.to_string(),
                        session_dir.to_path_buf(),
                        Arc::clone(&self.ledger_workers),
                        client,
                    )
                }),
            Err(error) => Err(error),
        };
        #[cfg(not(test))]
        self.install_session_db_handle(vm_id, db_path, opened, started)
    }

    /// The handle already registered for `vm_id` at `db_path`, if any. A
    /// handle for another path is left for the caller to replace.
    fn registered_session_db_handle(
        &self,
        vm_id: &str,
        db_path: &StdPath,
        started: std::time::Instant,
    ) -> Option<Arc<SessionLedger>> {
        let handle = self.session_db_handles.lock().unwrap().get(vm_id).cloned()?;
        if handle.path() == db_path {
            tracing::debug!(
                vm_id,
                db_path = %db_path.display(),
                operation = "register_session_db_handle",
                duration_ms = started.elapsed().as_millis(),
                "reused existing session DB handle"
            );
            return Some(handle);
        }
        warn!(
            vm_id,
            cached_db_path = %handle.path().display(),
            db_path = %db_path.display(),
            operation = "register_session_db_handle",
            "replacing session DB handle for rebound session path"
        );
        None
    }

    fn install_session_db_handle(
        &self,
        vm_id: &str,
        db_path: PathBuf,
        opened: Result<SessionLedger, String>,
        started: std::time::Instant,
    ) -> anyhow::Result<Arc<SessionLedger>> {
        let handle = match opened {
            Ok(handle) => Arc::new(handle),
            Err(error) => {
                error!(
                    vm_id,
                    db_path = %db_path.display(),
                    operation = "register_session_db_handle",
                    duration_ms = started.elapsed().as_millis(),
                    error = %error,
                    "failed to register session DB handle"
                );
                return Err(anyhow!(
                    "failed to open session DB handle for {vm_id}: {}: {error}",
                    db_path.display()
                ));
            }
        };
        self.session_db_handles
            .lock()
            .unwrap()
            .insert(vm_id.to_string(), Arc::clone(&handle));
        info!(
            vm_id,
            db_path = %db_path.display(),
            operation = "register_session_db_handle",
            duration_ms = started.elapsed().as_millis(),
            "registered session DB handle"
        );
        Ok(handle)
    }

    pub(crate) fn unregister_session_db_handle(&self, vm_id: &str) {
        let removed = self.session_db_handles.lock().unwrap().remove(vm_id);
        forget_session_responses(self, vm_id);
        if removed.is_some() {
            info!(
                vm_id,
                operation = "unregister_session_db_handle",
                "unregistered session DB handle"
            );
        }
    }

    #[cfg(test)]
    pub(crate) fn rename_session_db_handle(&self, old_vm_id: &str, new_vm_id: &str) {
        let mut handles = self.session_db_handles.lock().unwrap();
        if let Some(handle) = handles.remove(old_vm_id) {
            handles.insert(new_vm_id.to_string(), handle);
            drop(handles);
            info!(
                old_vm_id,
                new_vm_id,
                operation = "rename_session_db_handle",
                "renamed session DB handle"
            );
        }
    }

    pub(crate) fn session_db_handle(&self, vm_id: &str) -> Option<Arc<SessionLedger>> {
        self.session_db_handles.lock().unwrap().get(vm_id).cloned()
    }

    pub(crate) async fn hydrate_session_db_handles(&self) {
        let mut candidates: Vec<(String, PathBuf)> = {
            let instances = self.instances.lock().unwrap();
            instances
                .values()
                .map(|info| (info.id.clone(), info.session_dir.clone()))
                .collect()
        };
        {
            let registry = self.persistent_registry.lock().unwrap();
            candidates.extend(
                registry
                    .data
                    .vms
                    .values()
                    .map(|entry| (persistent_entry_vm_id(entry), entry.session_dir.clone())),
            );
        }

        let mut hydrated = 0usize;
        for (vm_id, session_dir) in candidates {
            let db_path = session_db_path_for_session_dir(&session_dir);
            if !db_path.exists() {
                info!(
                    vm_id,
                    operation = "hydrate_session_db_handle",
                    db_path = %db_path.display(),
                    "session DB absent during startup handle hydration"
                );
                continue;
            }
            match self.register_session_db_handle_async(&vm_id, &session_dir).await {
                Ok(_) => hydrated += 1,
                Err(error) => {
                    warn!(
                        vm_id,
                        operation = "hydrate_session_db_handle",
                        db_path = %db_path.display(),
                        error = %error,
                        "failed to hydrate session DB handle"
                    );
                }
            }
        }
        info!(
            operation = "hydrate_session_db_handles",
            hydrated, "startup session DB handle hydration complete"
        );
    }
}
