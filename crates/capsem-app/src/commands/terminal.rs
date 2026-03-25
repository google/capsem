use std::io::Write;
use std::sync::Arc;

use capsem_core::{HostToGuest, encode_host_msg, validate_host_msg};
use tauri::State;

use crate::boot::clone_fd;
use crate::state::AppState;
use super::{active_vm_id, vm_status_inner};

#[tauri::command]
pub fn vm_status(state: State<'_, AppState>) -> String {
    vm_status_inner(&state)
}

#[tauri::command]
pub async fn serial_input(input: String, state: State<'_, AppState>) -> Result<(), String> {
    tracing::debug!("Received serial input: {:?}", input.as_bytes());
    let vm_id = active_vm_id(&state)?;

    let tx = state.terminal_input_tx.clone();

    // Extract fd while holding the lock
    let fd = {
        let vms = state.vms.lock().unwrap();
        let instance = vms.get(&vm_id).ok_or("no VM running")?;
        instance.vsock_terminal_fd.unwrap_or(instance.serial_input_fd)
    };

    // Send the input to the dedicated background thread.
    // This is instant, non-blocking, and avoids spawning a Tokio thread per keystroke.
    tx.send((fd, input))
        .map_err(|e| format!("send to input queue failed: {e}"))
}

/// Poll for terminal output data. Blocks until data is available or the
/// terminal is closed. Returns bytes as a JSON array (Tauri serialization).
#[tauri::command]
pub async fn terminal_poll(state: State<'_, AppState>) -> Result<Vec<u8>, String> {
    let queue = Arc::clone(&state.terminal_output);
    queue.poll().await
        .ok_or_else(|| "terminal closed".to_string())
}

#[tauri::command]
pub async fn terminal_resize(
    cols: u16,
    rows: u16,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let vm_id = active_vm_id(&state)?;

    // Extract fd and state while holding the lock, then release before writing.
    // Same pattern as serial_input: avoid holding the mutex during blocking I/O.
    let (control_fd, host_state) = {
        let vms = state.vms.lock().unwrap();
        let instance = vms.get(&vm_id).ok_or("no VM running")?;
        let fd = instance.vsock_control_fd.ok_or("vsock control not connected")?;
        (fd, instance.state_machine.state())
    };

    let msg = HostToGuest::Resize { cols, rows };
    validate_host_msg(&msg, host_state)
        .map_err(|e| format!("{e}"))?;
    let frame = encode_host_msg(&msg).map_err(|e| format!("{e}"))?;

    let mut file = clone_fd(control_fd)
        .map_err(|e| format!("clone control fd failed: {e}"))?;
    tokio::task::spawn_blocking(move || {
        file.write_all(&frame)
            .map_err(|e| format!("control write failed: {e}"))
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}
