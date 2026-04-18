mod gateway;
mod icons;
mod menu;

use std::sync::mpsc;

use anyhow::{Context, Result};
use clap::Parser;
use muda::MenuEvent;
use tracing::{error, info, warn};
use tray_icon::TrayIconBuilder;

use crate::gateway::GatewayClient;
use crate::icons::TrayState;
use crate::menu::Action;

#[derive(Parser)]
#[command(about = "Capsem system tray")]
struct Args {
    /// Gateway port (overrides discovery from gateway.port file)
    #[arg(long)]
    port: Option<u16>,

    /// Poll interval in seconds
    #[arg(long, default_value = "5")]
    interval: u64,
}

/// Message from the async poller to the main thread.
enum PollResult {
    Status(gateway::StatusResponse),
    Unavailable(String),
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "capsem_tray=info".into()),
        )
        .init();

    let args = Args::parse();

    // Channel: async poller -> main thread (std mpsc is fine for non-tokio main thread)
    let (poll_tx, poll_rx) = mpsc::channel::<PollResult>();
    // Channel: main thread -> async runtime (tokio mpsc for async recv)
    let (action_tx, action_rx) = tokio::sync::mpsc::channel::<Action>(32);

    // Pre-decode icons
    let icon_idle = icons::load_icon(TrayState::Idle);
    let icon_error = icons::load_icon(TrayState::Error);

    // Initialize NSApplication as Accessory (no dock icon, but participates
    // in the event system so menu clicks are delivered).
    #[cfg(target_os = "macos")]
    let macos_app = {
        use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
        use objc2_foundation::MainThreadMarker;
        let mtm = MainThreadMarker::new().expect("must be on main thread");
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
        app.finishLaunching();
        app
    };

    // Build initial tray icon with idle state
    let initial_state = TrayState::Idle;
    let initial_menu = menu::build_unavailable_menu();

    let tray = TrayIconBuilder::new()
        .with_icon(icon_idle.clone())
        .with_icon_as_template(true)
        .with_menu(Box::new(initial_menu))
        .with_tooltip("Capsem")
        .build()
        .context("failed to build tray icon")?;

    info!(
        version = env!("CARGO_PKG_VERSION"),
        built = option_env!("CAPSEM_BUILD_TS").unwrap_or("dev"),
        "capsem-tray started"
    );
    eprintln!("[capsem-tray] version={} built={}", env!("CARGO_PKG_VERSION"), option_env!("CAPSEM_BUILD_TS").unwrap_or("dev"));
    info!("tray icon created");

    // Spawn tokio runtime on background thread
    let interval_secs = args.interval;
    let port_override = args.port;

    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to build tokio runtime");

        rt.block_on(async move {
            async_worker(port_override, interval_secs, poll_tx, action_rx).await;
        });
    });

    // macOS requires the menu event loop on the main thread.
    // tray-icon uses winit/tao-compatible event handling.
    let menu_channel = MenuEvent::receiver();

    let mut last_state = Some(initial_state);
    let mut last_status: Option<gateway::StatusResponse> = None;

    loop {
        // Process poll results (non-blocking)
        while let Ok(result) = poll_rx.try_recv() {
            match result {
                PollResult::Status(status) => {
                    // Icon always stays as template (white) -- only
                    // switch back from Error to Idle when gateway reconnects.
                    if last_state != Some(TrayState::Idle) {
                        tray.set_icon_with_as_template(Some(icon_idle.clone()), true)
                            .unwrap_or_else(|e| warn!("failed to set icon: {e}"));
                        last_state = Some(TrayState::Idle);
                    }

                    if last_status.as_ref() != Some(&status) {
                        let new_menu = menu::build_menu(&status);
                        tray.set_menu(Some(Box::new(new_menu)));
                        last_status = Some(status);
                    }
                }
                PollResult::Unavailable(reason) => {
                    if last_state != Some(TrayState::Error) {
                        tray.set_icon_with_as_template(Some(icon_error.clone()), true)
                            .unwrap_or_else(|e| warn!("failed to set icon: {e}"));
                        tray.set_menu(Some(Box::new(menu::build_unavailable_menu())));
                        last_state = Some(TrayState::Error);
                        last_status = None;
                    }

                    warn!("gateway unavailable: {reason}");
                }
            }
        }

        // Process menu clicks
        while let Ok(event) = menu_channel.try_recv() {
            let action = menu::parse_action(&event.id);
            info!(menu_id = %event.id.0, action = ?action, "menu click");
            match action {
                Some(Action::Quit) => {
                    info!("quit requested");
                    return Ok(());
                }
                Some(Action::OpenUi) => {
                    launch_ui(None);
                }
                Some(action) => {
                    if action_tx.blocking_send(action).is_err() {
                        error!("async worker gone, exiting");
                        return Ok(());
                    }
                }
                None => {}
            }
        }

        // Drain pending NSEvents so macOS delivers status item clicks, menu
        // popups, and redraws. CFRunLoopRunInMode alone doesn't drive
        // NSApplication event dispatch. We pull events until none remain,
        // then sleep 16ms to avoid busy-spinning (~60 Hz).
        #[cfg(target_os = "macos")]
        {
            use objc2_app_kit::NSEventMask;
            use objc2_foundation::NSDate;
            let until = NSDate::dateWithTimeIntervalSinceNow(0.016);
            loop {
                let mode = unsafe { objc2_foundation::NSDefaultRunLoopMode };
                let event = macos_app.nextEventMatchingMask_untilDate_inMode_dequeue(
                    NSEventMask::Any,
                    Some(&until),
                    mode,
                    true,
                );
                match event {
                    Some(event) => macos_app.sendEvent(&event),
                    None => break,
                }
            }
        }
        #[cfg(not(target_os = "macos"))]
        std::thread::sleep(std::time::Duration::from_millis(16));
    }
}

async fn async_worker(
    port_override: Option<u16>,
    interval_secs: u64,
    poll_tx: mpsc::Sender<PollResult>,
    mut action_rx: tokio::sync::mpsc::Receiver<Action>,
) {
    let interval_duration = std::time::Duration::from_secs(interval_secs);
    let mut poll_interval = tokio::time::interval(interval_duration);

    // Initial discovery -- retry until gateway is reachable
    let mut client = loop {
        match GatewayClient::discover(port_override).await {
            Ok(c) => break c,
            Err(e) => {
                warn!("gateway discovery failed: {e}");
                let _ = poll_tx.send(PollResult::Unavailable(e.to_string()));
                tokio::time::sleep(interval_duration).await;
            }
        }
    };

    info!("gateway discovered at port {}", client.port());

    loop {
        tokio::select! {
            _ = poll_interval.tick() => {
                // Poll status
                match client.status().await {
                    Ok(status) => {
                        let _ = poll_tx.send(PollResult::Status(status));
                    }
                    Err(e) => {
                        warn!("status poll failed: {e}");
                        // Try re-discovery (token/port may have changed)
                        match GatewayClient::discover(port_override).await {
                            Ok(new_client) => {
                                client = new_client;
                                info!("gateway re-discovered at port {}", client.port());
                            }
                            Err(_) => {
                                let _ = poll_tx.send(PollResult::Unavailable(e.to_string()));
                            }
                        }
                    }
                }
            }
            Some(action) = action_rx.recv() => {
                dispatch_action(&client, action).await;
                // After an action, trigger an immediate status poll to update UI
                poll_interval.reset(); // Optional: reset interval if we want to delay next poll
                // OR just poll immediately:
                if let Ok(status) = client.status().await {
                     let _ = poll_tx.send(PollResult::Status(status));
                }
            }
        }
    }
}

async fn dispatch_action(client: &GatewayClient, action: Action) {
    info!(action = ?action, "dispatching tray action");
    let result = match &action {
        Action::Connect(id) => {
            launch_ui(Some(id));
            return;
        }
        Action::Stop(id) => {
            let r = client.stop_vm(id).await;
            info!(id = %id, ok = r.is_ok(), "stop_vm");
            r
        }
        Action::Delete(id) => {
            let r = client.delete_vm(id).await;
            info!(id = %id, ok = r.is_ok(), "delete_vm");
            r
        }
        Action::Suspend(id) => {
            let r = client.suspend_vm(id).await;
            info!(id = %id, ok = r.is_ok(), "suspend_vm");
            r
        }
        Action::Resume(id) => {
            let r = client.resume_vm(id).await;
            info!(id = %id, ok = r.is_ok(), "resume_vm");
            r
        }
        Action::NewSession => {
            info!("provisioning new temp session");
            match client.provision_temp().await {
                Ok(id) => {
                    info!(id = %id, "new session provisioned, launching UI");
                    launch_ui(Some(&id));
                    return;
                }
                Err(e) => Err(e),
            }
        }
        Action::Save(id) => {
            launch_ui_action(id, "save");
            return;
        }
        Action::Fork(id) => {
            launch_ui_action(id, "fork");
            return;
        }
        Action::OpenUi | Action::Quit => return,
    };

    if let Err(e) = result {
        error!("action {action:?} failed: {e}");
    }
}

fn launch_ui(vm_id: Option<&str>) {
    info!(vm_id = ?vm_id, "launching Capsem.app");

    // On macOS, `open -a Capsem --args X` only delivers X when the app is
    // NOT already running. When it is, `open` just brings the existing
    // window to front and the args are dropped -- the single-instance
    // plugin callback never fires because no second process spawns.
    //
    // Invoking the binary path directly always spawns a process. If Capsem
    // is already running, tauri-plugin-single-instance detects the second
    // launch, forwards argv to the running instance via its IPC socket,
    // and the new process exits. If not, the new process becomes the
    // primary instance.
    #[cfg(target_os = "macos")]
    let binary_candidates: [std::path::PathBuf; 2] = [
        std::path::PathBuf::from("/Applications/Capsem.app/Contents/MacOS/capsem-app"),
        std::env::var("HOME")
            .map(|h| std::path::PathBuf::from(h).join("Applications/Capsem.app/Contents/MacOS/capsem-app"))
            .unwrap_or_default(),
    ];
    #[cfg(target_os = "linux")]
    let binary_candidates: [std::path::PathBuf; 2] = [
        std::path::PathBuf::from("/usr/bin/capsem-app"),
        std::path::PathBuf::from("/usr/local/bin/capsem-app"),
    ];
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let binary_candidates: [std::path::PathBuf; 0] = [];

    let binary = binary_candidates.iter().find(|p| p.exists());

    let mut cmd = if let Some(path) = binary {
        std::process::Command::new(path)
    } else {
        // Fallback for uninstalled dev environments: LaunchServices lookup.
        // Works only when the app isn't already running.
        warn!("capsem-app binary not found in install locations; falling back to `open -a Capsem`");
        let mut c = std::process::Command::new("open");
        c.args(["-a", "Capsem"]);
        if vm_id.is_some() {
            c.arg("--args");
        }
        c
    };

    if let Some(id) = vm_id {
        cmd.args(["--connect", id]);
    }

    match cmd.spawn() {
        Ok(_) => info!(vm_id = ?vm_id, "Capsem.app launch dispatched"),
        Err(e) => warn!("failed to launch UI: {e}"),
    }
}

/// Launch the UI with a VM deep-link plus an action hint (e.g. "save", "fork").
/// The UI opens the VM tab and dispatches the action (opens the relevant modal)
/// -- actions requiring a user-supplied name belong in the UI, not the tray.
fn launch_ui_action(vm_id: &str, action: &str) {
    info!(%vm_id, %action, "launching Capsem.app for action");

    #[cfg(target_os = "macos")]
    let binary_candidates: [std::path::PathBuf; 2] = [
        std::path::PathBuf::from("/Applications/Capsem.app/Contents/MacOS/capsem-app"),
        std::env::var("HOME")
            .map(|h| std::path::PathBuf::from(h).join("Applications/Capsem.app/Contents/MacOS/capsem-app"))
            .unwrap_or_default(),
    ];
    #[cfg(target_os = "linux")]
    let binary_candidates: [std::path::PathBuf; 2] = [
        std::path::PathBuf::from("/usr/bin/capsem-app"),
        std::path::PathBuf::from("/usr/local/bin/capsem-app"),
    ];
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let binary_candidates: [std::path::PathBuf; 0] = [];

    let binary = binary_candidates.iter().find(|p| p.exists());
    let mut cmd = if let Some(path) = binary {
        std::process::Command::new(path)
    } else {
        warn!("capsem-app binary not found; falling back to `open -a Capsem`");
        let mut c = std::process::Command::new("open");
        c.args(["-a", "Capsem", "--args"]);
        c
    };
    cmd.args(["--connect", vm_id, "--action", action]);
    match cmd.spawn() {
        Ok(_) => info!(%vm_id, %action, "UI action dispatched"),
        Err(e) => warn!("failed to launch UI: {e}"),
    }
}

