use std::io::Write;

use capsem_core::vsock;
use tauri::State;

use crate::borrow_fd;
use crate::state::AppState;

/// Default VM ID for the single-VM case.
const DEFAULT_VM_ID: &str = "default";

#[tauri::command]
pub fn vm_status(state: State<'_, AppState>) -> String {
    let vms = state.vms.lock().unwrap();
    match vms.get(DEFAULT_VM_ID) {
        Some(instance) => instance.vm.state().to_string(),
        None => "not created".to_string(),
    }
}

#[tauri::command]
pub fn serial_input(input: String, state: State<'_, AppState>) -> Result<(), String> {
    tracing::debug!("Received serial input: {:?}", input.as_bytes());
    let vms = state.vms.lock().unwrap();
    let instance = vms.get(DEFAULT_VM_ID).ok_or("no VM running")?;

    // Prefer vsock terminal if connected, fall back to serial.
    let fd = instance.vsock_terminal_fd.unwrap_or(instance.serial_input_fd);

    // Safety: fd is valid for the lifetime of the VM.
    let mut file = unsafe { borrow_fd(fd) };
    file.write_all(input.as_bytes())
        .map_err(|e| format!("write failed: {e}"))
}

#[tauri::command]
pub fn terminal_resize(
    cols: u16,
    rows: u16,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let vms = state.vms.lock().unwrap();
    let instance = vms.get(DEFAULT_VM_ID).ok_or("no VM running")?;

    let control_fd = instance.vsock_control_fd.ok_or("vsock control not connected")?;

    let msg = vsock::ControlMessage::Resize { cols, rows };
    let frame = vsock::encode_control_message(&msg).map_err(|e| format!("{e}"))?;

    // Safety: fd is valid for the lifetime of the VM.
    let mut file = unsafe { borrow_fd(control_fd) };
    file.write_all(&frame)
        .map_err(|e| format!("control write failed: {e}"))
}
