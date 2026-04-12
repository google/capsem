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
    let icon_active = icons::load_icon(TrayState::Active);
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
                    let state = state_for_status(&status);
                    
                    if last_state != Some(state) {
                        let is_template = state == TrayState::Idle;
                        let icon = match state {
                            TrayState::Idle => icon_idle.clone(),
                            TrayState::Active => icon_active.clone(),
                            TrayState::Error => icon_error.clone(),
                        };
                        tray.set_icon_with_as_template(Some(icon), is_template)
                            .unwrap_or_else(|e| warn!("failed to set icon: {e}"));
                        last_state = Some(state);
                    }

                    if last_status.as_ref() != Some(&status) {
                        let new_menu = menu::build_menu(&status);
                        tray.set_menu(Some(Box::new(new_menu)));
                        last_status = Some(status);
                    }
                }
                PollResult::Unavailable(reason) => {
                    let state = TrayState::Error;
                    
                    if last_state != Some(state) {
                        tray.set_icon_with_as_template(Some(icon_error.clone()), false)
                            .unwrap_or_else(|e| warn!("failed to set icon: {e}"));
                        tray.set_menu(Some(Box::new(menu::build_unavailable_menu())));
                        last_state = Some(state);
                        last_status = None;
                    }
                    
                    warn!("gateway unavailable: {reason}");
                }
            }
        }

        // Process menu clicks
        while let Ok(event) = menu_channel.try_recv() {
            match menu::parse_action(&event.id) {
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
    let result = match &action {
        Action::Connect(id) => {
            launch_ui(Some(id));
            return;
        }
        Action::Stop(id) => client.stop_vm(id).await,
        Action::Delete(id) => client.delete_vm(id).await,
        Action::Suspend(id) => client.suspend_vm(id).await,
        Action::Resume(id) => client.resume_vm(id).await,
        Action::Fork(id) => client.fork_vm(id).await,
        Action::NewTemp => {
            match client.provision_temp().await {
                Ok(id) => {
                    launch_ui(Some(&id));
                    return;
                }
                Err(e) => Err(e),
            }
        }
        Action::NewNamed => {
            // Named VMs need a name -- the tray can't prompt, so open the UI
            // which has a dialog for naming. Pass --new-named flag.
            launch_ui_new_named();
            return;
        }
        Action::OpenUi | Action::Quit => return,
    };

    if let Err(e) = result {
        error!("action {action:?} failed: {e}");
    }
}

/// Determine tray icon state from a status response.
fn state_for_status(status: &gateway::StatusResponse) -> TrayState {
    if status.vm_count > 0 {
        TrayState::Active
    } else {
        TrayState::Idle
    }
}

fn launch_ui(vm_id: Option<&str>) {
    let mut cmd = std::process::Command::new("open");
    cmd.args(["-a", "Capsem"]);
    if let Some(id) = vm_id {
        cmd.args(["--args", "--connect", id]);
    }
    if let Err(e) = cmd.spawn() {
        warn!("failed to launch UI: {e}");
    }
}

fn launch_ui_new_named() {
    let mut cmd = std::process::Command::new("open");
    cmd.args(["-a", "Capsem", "--args", "--new-named"]);
    if let Err(e) = cmd.spawn() {
        warn!("failed to launch UI: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::{StatusResponse, VmSummary};

    fn make_status(vm_count: u32, vms: Vec<VmSummary>) -> StatusResponse {
        StatusResponse {
            service: "running".into(),
            vm_count,
            vms,
            latency_ms: Some(5),
        }
    }

    fn make_vm(id: &str, status: &str) -> VmSummary {
        VmSummary {
            id: id.into(),
            name: None,
            status: status.into(),
            persistent: false,
        }
    }

    #[test]
    fn state_active_when_vms_running() {
        let status = make_status(2, vec![make_vm("a", "running"), make_vm("b", "running")]);
        assert_eq!(state_for_status(&status), TrayState::Active);
    }

    #[test]
    fn state_idle_when_no_vms() {
        let status = make_status(0, vec![]);
        assert_eq!(state_for_status(&status), TrayState::Idle);
    }

    #[test]
    fn state_active_with_one_vm() {
        let status = make_status(1, vec![make_vm("x", "suspended")]);
        assert_eq!(state_for_status(&status), TrayState::Active);
    }
}
