#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(not(any(test, clippy)))]
fn context() -> tauri::Context<tauri::Wry> {
    tauri::generate_context!()
}

// Tauri's test context validates configuration and generated code without
// reading frontendDist. Clippy still checks the real library-owned runtime;
// only the zero-logic asset embedding expression is substituted.
#[cfg(any(test, clippy))]
fn context() -> tauri::Context<tauri::Wry> {
    tauri::generate_context!(assets = tauri::test::noop_assets(), test = true)
}

fn main() {
    capsem_app::run(context());
}
