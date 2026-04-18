#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::Serialize;
use tauri::{Emitter, Manager};
use tracing::{info, warn};
use tracing_subscriber::prelude::*;
use tracing_subscriber::{fmt::format::FmtSpan, EnvFilter};

// ---------- IPC commands ----------

#[tauri::command]
fn log_frontend(level: String, message: String) {
    eprintln!("[frontend:{level}] {message}");
    match level.as_str() {
        "error" => tracing::error!(target: "frontend", "{message}"),
        "warn" => tracing::warn!(target: "frontend", "{message}"),
        "info" => tracing::info!(target: "frontend", "{message}"),
        _ => tracing::debug!(target: "frontend", "{message}"),
    }
}

#[tauri::command]
async fn open_url(url: String, app: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url(&url, None::<&str>)
        .map_err(|e| e.to_string())
}

#[derive(Serialize)]
struct UpdateInfo {
    version: String,
    current_version: String,
}

#[tauri::command]
async fn check_for_app_update(app: tauri::AppHandle) -> Result<Option<UpdateInfo>, String> {
    use tauri_plugin_updater::UpdaterExt;
    let updater = app.updater().map_err(|e| format!("updater unavailable: {e}"))?;
    let update = updater.check().await.map_err(|e| format!("update check failed: {e}"))?;
    Ok(update.map(|u| UpdateInfo {
        version: u.version.clone(),
        current_version: app.package_info().version.to_string(),
    }))
}

// ---------- Deep link handling (--connect <vm_id>) ----------

fn parse_flag(args: &[String], flag: &str) -> Option<String> {
    let mut i = 0;
    while i < args.len() {
        if args[i] == flag && i + 1 < args.len() {
            return Some(args[i + 1].clone());
        }
        i += 1;
    }
    None
}

fn parse_connect_arg(args: &[String]) -> Option<String> {
    parse_flag(args, "--connect")
}

fn parse_action_arg(args: &[String]) -> Option<String> {
    parse_flag(args, "--action")
}

fn dispatch_deep_link(window: &tauri::WebviewWindow, vm_id: &str, action: Option<&str>) {
    let escaped_id = vm_id.replace('\'', "\\'");
    let action_part = action
        .map(|a| format!(", action: '{}'", a.replace('\'', "\\'")))
        .unwrap_or_default();
    let _ = window.eval(&format!(
        "if (window.__capsemDeepLink) {{ window.__capsemDeepLink({{ connect: '{escaped_id}'{action_part} }}) }}"
    ));
}

// ---------- Auto-update dialog ----------

async fn check_for_update_with_prompt(app: tauri::AppHandle) {
    use tauri_plugin_dialog::DialogExt;
    use tauri_plugin_updater::UpdaterExt;

    let Ok(updater) = app.updater() else { return };
    let update = match updater.check().await {
        Ok(Some(u)) => u,
        Ok(None) => return,
        Err(e) => {
            info!("update check failed: {e:#}");
            return;
        }
    };

    let current = app.package_info().version.to_string();
    let accepted = app
        .dialog()
        .message(format!(
            "Capsem {} is available (you have {current}). Download and install?",
            update.version
        ))
        .title("Update Available")
        .buttons(tauri_plugin_dialog::MessageDialogButtons::OkCancel)
        .blocking_show();
    if !accepted {
        return;
    }
    if let Err(e) = update.download_and_install(|_, _| {}, || {}).await {
        tracing::error!("update failed: {e:#}");
    } else {
        app.restart();
    }
}

// ---------- Log housekeeping ----------

fn cleanup_old_logs(dir: &Path, max_days: u64) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let now = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let cutoff = now.saturating_sub(max_days * 86400);
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() { continue }
        let Ok(modified) = meta.modified() else { continue };
        let Ok(mtime) = modified.duration_since(std::time::UNIX_EPOCH) else { continue };
        if mtime.as_secs() < cutoff {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

fn log_filename() -> String {
    let secs = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let t = secs % 86400;
    let days = secs / 86400;
    let z = days as i64 + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}-{:02}-{:02}.jsonl",
        t / 3600,
        (t % 3600) / 60,
        t % 60
    )
}

fn main() {
    // Log to ~/.capsem/logs/<timestamp>.jsonl
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    let log_dir = PathBuf::from(&home).join(".capsem").join("logs");
    let _ = std::fs::create_dir_all(&log_dir);
    cleanup_old_logs(&log_dir, 7);

    let log_path = log_dir.join(log_filename());
    let file_layer = std::fs::File::create(&log_path).ok().map(|f| {
        let (nb, guard) = tracing_appender::non_blocking(f);
        // Leak the guard — we want logs flushed for the entire process lifetime.
        Box::leak(Box::new(guard));
        tracing_subscriber::fmt::layer()
            .json()
            .with_writer(nb)
            .with_span_events(FmtSpan::CLOSE)
    });

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("capsem_ui=info,frontend=info"));

    let stdout_layer = tracing_subscriber::fmt::layer().with_span_events(FmtSpan::CLOSE);

    tracing_subscriber::registry()
        .with(filter)
        .with(stdout_layer)
        .with(file_layer)
        .init();

    let cli_args: Vec<String> = std::env::args().skip(1).collect();
    info!(
        version = env!("CARGO_PKG_VERSION"),
        built = option_env!("CAPSEM_BUILD_TS").unwrap_or("dev"),
        args = ?cli_args,
        "starting capsem-ui"
    );

    let connect_id = parse_connect_arg(&cli_args);
    let initial_action = parse_action_arg(&cli_args);

    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            info!(args = ?args, "single-instance: second launch");
            let Some(window) = app.get_webview_window("main") else {
                warn!("single-instance: main window missing");
                return;
            };
            let _ = window.set_focus();
            if let Some(id) = parse_connect_arg(&args) {
                let action = parse_action_arg(&args);
                dispatch_deep_link(&window, &id, action.as_deref());
            }
        }))
        .setup(move |app| {
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                check_for_update_with_prompt(handle).await;
            });

            if let Some(id) = connect_id.clone() {
                let action = initial_action.clone();
                let window = app
                    .get_webview_window("main")
                    .expect("main window must exist");
                tauri::async_runtime::spawn(async move {
                    // Let the frontend mount __capsemDeepLink before dispatching.
                    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                    dispatch_deep_link(&window, &id, action.as_deref());
                });
            }

            // Emit an init event for the frontend so it can detect Tauri context.
            let _ = app.handle().emit("capsem-ready", ());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            log_frontend,
            open_url,
            check_for_app_update,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
