//! Immutable application identity retained with the launched workload.
use super::*;

impl ContainerSetups {
    pub(super) fn set_app(&self, id: &str, generation: u64, app: Option<api::ImageApp>) -> bool {
        let mut records = self.records.lock().unwrap();
        let Some(record) = records.get_mut(id).filter(|record| record.generation == generation) else {
            return false;
        };
        record.app = app;
        drop(records);
        true
    }
}

/// Source identity belongs to the workload owner/launch record, never a
/// session-name guess or a fresh lookup of the catalog's newest version.
/// Called on the service blocking worker for the recorded fallback.
pub(crate) fn source_app(state: &ServiceState, id: &str, session_dir: &std::path::Path) -> Option<api::ImageApp> {
    {
        let records = state.containers.records.lock().unwrap();
        if let Some(record) = records.get(id) {
            return record
                .app
                .clone()
                .filter(|app| record.status.resolved.as_deref() == Some(&app.image));
        }
    }
    let record = read_launch_record(session_dir)?;
    record.app.filter(|app| record.resolved.as_deref() == Some(&app.image))
}
