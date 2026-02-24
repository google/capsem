use std::io::Write;
use std::os::unix::io::FromRawFd;

use tauri::State;

use crate::state::AppState;

#[tauri::command]
pub fn vm_status(state: State<'_, AppState>) -> String {
    let vm = state.vm.lock().unwrap();
    match vm.as_ref() {
        Some(vm) => vm.state().to_string(),
        None => "not created".to_string(),
    }
}

#[tauri::command]
pub fn serial_input(input: String, state: State<'_, AppState>) -> Result<(), String> {
    let guard = state.serial_input_fd.lock().unwrap();
    let fd = guard.ok_or_else(|| "no serial input fd".to_string())?;
    // Safety: fd is a valid pipe write end, kept open for the lifetime of the app.
    // We create a File but use ManuallyDrop to avoid closing the fd when dropped.
    let mut file = std::mem::ManuallyDrop::new(unsafe { std::fs::File::from_raw_fd(fd) });
    file.write_all(input.as_bytes())
        .map_err(|e| format!("serial write failed: {e}"))
}
