//! Authenticated per-session ledger channels held by service routes.

use super::*;
use capsem_logger::ledger_protocol::{LedgerQuery, LedgerRows};
#[cfg(not(test))]
use capsem_proto::ledger::LedgerClientRole;
use capsem_proto::ledger_counters::LedgerCounters;

pub(super) fn session_db_path_for_session_dir(session_dir: &StdPath) -> PathBuf {
    session_dir.join("session.db")
}

pub(crate) struct SessionLedger {
    path: PathBuf,
    client: Option<capsem_logger::ledger_client::LedgerClient>,
    #[cfg(test)]
    embedded: Option<capsem_logger::DbHandle>,
}

impl SessionLedger {
    #[cfg(not(test))]
    fn remote(path: PathBuf, client: capsem_logger::ledger_client::LedgerClient) -> Self {
        Self {
            path,
            client: Some(client),
            #[cfg(test)]
            embedded: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn embedded(path: PathBuf, db: capsem_logger::DbHandle) -> Self {
        Self {
            path,
            client: None,
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
        self.client().counters().await.map(|_| ())
    }

    pub(crate) async fn query(&self, query: LedgerQuery) -> Result<Vec<LedgerRows>, String> {
        #[cfg(test)]
        if let Some(db) = &self.embedded {
            return capsem_logger::ledger_server::execute_named_query(db, query).await;
        }
        self.client().query(query).await
    }

    pub(crate) async fn ledger_counters(&self) -> Result<Arc<LedgerCounters>, String> {
        #[cfg(test)]
        if let Some(db) = &self.embedded {
            return db.ledger_counters().await;
        }
        self.client().counters().await
    }

    pub(crate) fn read_cache_epoch(&self, _domain: capsem_logger::ReadCacheDomain) -> u64 {
        #[cfg(test)]
        if let Some(db) = &self.embedded {
            return db.read_cache_epoch(capsem_logger::ReadCacheDomain::All);
        }
        self.client().read_cache_epoch()
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
        self.client().read_bodies(event_id).await
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
        Ok(SessionWarcExport::Remote(self.client().export_warc().await?))
    }

    fn client(&self) -> &capsem_logger::ledger_client::LedgerClient {
        self.client.as_ref().expect("production session ledger has a client")
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
                let (stream, grant) = channel.into_parts();
                (stream, grant)
            });
        #[cfg(not(test))]
        let opened = match opened {
            Ok((stream, grant)) => capsem_logger::ledger_client::LedgerClient::connect(stream, grant, db_path.clone())
                .await
                .map(|client| SessionLedger::remote(db_path.clone(), client)),
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
