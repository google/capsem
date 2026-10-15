use super::*;

pub(crate) async fn wait_for_vm_ready(
    uds_path: &std::path::Path,
    timeout_secs: u64,
    state: Option<&Arc<ServiceState>>,
    id: Option<&str>,
) -> Result<(), String> {
    let ready_span = tracing::debug_span!(
        target: "capsem.launch",
        capsem_foundation::telemetry::LAUNCH_VSOCK_READY_SPAN,
        status = tracing::field::Empty,
    );
    let ready_path = uds_path.with_extension("ready");
    // Override the PollOpts::new defaults (50ms / 500ms): VM ready-time is
    // sub-second in the common case and the sentinel check is a single stat,
    // so 500ms max_delay overshoots readiness by ~500ms and blows the
    // exec_ready / boot_ready latency gates. Peer callers (service-connect,
    // gateway-ready) wait for remote processes with seconds-scale startup
    // where 500ms is appropriate; this poll is different.
    let opts = vm_ready_poll_opts(timeout_secs);
    let died: Arc<std::sync::atomic::AtomicBool> = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let res = capsem_foundation::poll::poll_until(opts, || {
        let ready = ready_path.clone();
        let state = state.cloned();
        let id = id.map(|s| s.to_string());
        let died = Arc::clone(&died);
        async move {
            if super::launch::stamped(&ready, "ready") {
                return Some(());
            }
            if let (Some(st), Some(name)) = (state.as_ref(), id.as_ref()) {
                if !st.instances.lock().unwrap().contains_key(name) {
                    died.store(true, std::sync::atomic::Ordering::Release);
                    // Returning Some short-circuits the poll loop; the
                    // outer caller distinguishes via `died`.
                    return Some(());
                }
            }
            None
        }
    })
    .instrument(ready_span.clone())
    .await;
    if died.load(std::sync::atomic::Ordering::Acquire) {
        ready_span.record("status", "error");
        return Err("capsem-process exited before signalling ready".into());
    }
    match res {
        Ok(()) => {
            if let Some(state) = state {
                crate::credential_routes::sync_memory(state, uds_path).await?;
            }
            ready_span.record("status", "ok");
            Ok(())
        }
        Err(error) => {
            ready_span.record("status", "error");
            Err(format!("{error}"))
        }
    }
}

pub(crate) fn vm_ready_poll_opts(timeout_secs: u64) -> capsem_foundation::poll::PollOpts {
    capsem_foundation::poll::PollOpts {
        initial_delay: std::time::Duration::from_millis(5),
        max_delay: std::time::Duration::from_millis(50),
        ..capsem_foundation::poll::PollOpts::new("vm-ready", std::time::Duration::from_secs(timeout_secs))
    }
}
