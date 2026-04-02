use std::collections::HashMap;
use std::os::unix::io::RawFd;
use std::sync::{Arc, Mutex};

pub use capsem_core::{
    SandboxInstance as VmInstance,
    TerminalOutputQueue,
    VmState,
};
use capsem_core::log_layer::LogHandle;
use capsem_core::session::SessionIndex;

pub struct AppState {
    pub vms: Mutex<HashMap<String, VmInstance>>,
    pub session_index: Mutex<SessionIndex>,
    pub active_session_id: Mutex<Option<String>>,
    pub terminal_output: Arc<TerminalOutputQueue>,
    pub terminal_input_tx: std::sync::mpsc::Sender<(RawFd, String)>,
    pub log_handle: Option<LogHandle>,
    /// App-level lifecycle status, independent of any VM instance.
    /// Tracks states like "downloading" that exist before a VM is created.
    /// Valid values: "not created", "downloading", "booting", "running", "error".
    pub app_status: Mutex<String>,
    /// MCP gateway config (set after boot, provides access to snapshot scheduler + tools).
    pub mcp_config: Mutex<Option<Arc<capsem_core::mcp::gateway::McpGatewayConfig>>>,
}

impl AppState {
    pub fn new(session_index: SessionIndex, log_handle: Option<LogHandle>) -> Self {
        let (tx, rx) = std::sync::mpsc::channel::<(RawFd, String)>();
        
        // Spawn a dedicated global thread for batching terminal input writes.
        // This prevents spawning a new Tokio thread per character typed,
        // which causes severe CPU spikes and thread pool exhaustion.
        std::thread::spawn(move || {
            use std::io::Write;
            let mut current_fd: Option<RawFd> = None;
            let mut current_file: Option<std::fs::File> = None;

            /// Max bytes to coalesce before flushing to the PTY. Prevents
            /// unbounded memory growth if the channel is flooded faster than
            /// the inner try_recv loop can drain.
            const MAX_BATCH_SIZE: usize = 64 * 1024;

            while let Ok((fd, data)) = rx.recv() {
                let mut buf = data.into_bytes();
                // Coalesce rapid sequential inputs for the same file descriptor,
                // up to MAX_BATCH_SIZE to guarantee forward progress.
                while buf.len() < MAX_BATCH_SIZE {
                    match rx.try_recv() {
                        Ok((next_fd, next_data)) if next_fd == fd => {
                            buf.extend(next_data.into_bytes());
                        }
                        Ok(_) => {
                            // Very rare: active VM switched in the middle of a
                            // microsecond burst. Handled in the next iteration.
                            break;
                        }
                        Err(_) => break,
                    }
                }
                
                // Reuse the file handle if it's the same FD.
                if current_fd != Some(fd) {
                    current_fd = Some(fd);
                    current_file = crate::boot::clone_fd(fd).ok();
                }

                if let Some(mut file) = current_file.as_ref() {
                    let _ = file.write_all(&buf);
                    let _ = file.flush();
                }
            }
        });

        Self {
            vms: Mutex::new(HashMap::new()),
            session_index: Mutex::new(session_index),
            active_session_id: Mutex::new(None),
            terminal_output: Arc::new(TerminalOutputQueue::new()),
            terminal_input_tx: tx,
            log_handle,
            app_status: Mutex::new(VmState::NotCreated.to_string()),
            mcp_config: Mutex::new(None),
        }
    }
}
