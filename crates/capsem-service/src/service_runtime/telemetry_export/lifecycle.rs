//! Own synchronous SDK construction and shutdown off the service reactor.
use capsem_telemetry::export::{Exporter, Meter};

pub(crate) struct RuntimeExporter(Option<Exporter>);

impl RuntimeExporter {
    pub(crate) fn meter(&self) -> Meter {
        self.0.as_ref().expect("live exporter").meter()
    }

    pub(crate) async fn shutdown(mut self) {
        if let Some(exporter) = self.0.take() {
            let _ = tokio::task::spawn_blocking(move || drop(exporter)).await;
        }
    }
}

impl Drop for RuntimeExporter {
    fn drop(&mut self) {
        let Some(exporter) = self.0.take() else {
            return;
        };
        // Early startup errors/cancellation must use the same off-worker
        // destruction as normal shutdown. Tokio drains its blocking pool when
        // the owning runtime exits, even if this task handle is discarded.
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn_blocking(move || drop(exporter));
        } else {
            drop(exporter);
        }
    }
}

pub(crate) async fn install() -> Option<RuntimeExporter> {
    // The blocking HTTP client builds an internal runtime; both its creation
    // and every failure-path destructor must stay outside an async context.
    // Wrap before returning from the worker so cancelled joins are safe too.
    match tokio::task::spawn_blocking(|| super::install_sync().map(|exporter| RuntimeExporter(Some(exporter)))).await {
        Ok(exporter) => exporter,
        Err(_) => {
            tracing::warn!("optional metric exporter initialization failed");
            None
        }
    }
}
