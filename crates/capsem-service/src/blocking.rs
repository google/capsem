//! Blocking work leaves the async runtime through one door.
//!
//! Route handlers run on tokio workers. A filesystem read, a registry save,
//! a directory listing or a child spawn on that thread stalls every other
//! request until it returns; the UI polls list and status routes several
//! times a second, so one slow disk turned into a slow service. Every such
//! call goes through `off_worker`, and the source contract in
//! `tests/async_io_contract.rs` refuses the direct calls.

use super::*;

impl ServiceState {
    /// Run `f` on the blocking pool with a handle to the state.
    pub(crate) async fn off_worker<T, F>(self: &Arc<Self>, f: F) -> Result<T, AppError>
    where
        T: Send + 'static,
        F: FnOnce(Arc<ServiceState>) -> T + Send + 'static,
    {
        let state = Arc::clone(self);
        // Tests point profile lookups at a per-test directory through a
        // thread-local; the blocking thread has to see the caller's value.
        #[cfg(test)]
        let profile_dir_override = test_profile_dir_override();
        tokio::task::spawn_blocking(move || {
            #[cfg(test)]
            let previous = set_test_profile_dir_override(profile_dir_override);
            let result = f(state);
            #[cfg(test)]
            set_test_profile_dir_override(previous);
            result
        })
        .await
        .map_err(|error| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("blocking task failed: {error}"),
            )
        })
    }

    /// Re-read one profile's rule files after a mutation, off the worker.
    pub(crate) async fn refresh_profile_rule_cache_off_worker(
        self: &Arc<Self>,
        profile_id: String,
    ) -> Result<(), AppError> {
        self.off_worker(move |state| state.refresh_profile_rule_cache(Some(&profile_id)))
            .await?
            .map_err(|error| AppError(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
    }
}
