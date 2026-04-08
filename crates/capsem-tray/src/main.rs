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

    // Channel: async poller -> main thread
    let (poll_tx, poll_rx) = mpsc::channel::<PollResult>();
    // Channel: main thread -> async runtime (for actions)
    let (action_tx, action_rx) = mpsc::channel::<Action>();

    // Build initial tray icon with idle state
    let initial_icon = icons::load_icon(TrayState::Idle);
    let initial_menu = menu::build_unavailable_menu();

    let tray = TrayIconBuilder::new()
        .with_icon(initial_icon)
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

    loop {
        // Process poll results (non-blocking)
        while let Ok(result) = poll_rx.try_recv() {
            match result {
                PollResult::Status(status) => {
                    let state = if status.vm_count > 0 {
                        TrayState::Active
                    } else {
                        TrayState::Idle
                    };
                    tray.set_icon(Some(icons::load_icon(state)))
                        .unwrap_or_else(|e| warn!("failed to set icon: {e}"));
                    let new_menu = menu::build_menu(&status);
                    tray.set_menu(Some(Box::new(new_menu)));
                }
                PollResult::Unavailable(reason) => {
                    tray.set_icon(Some(icons::load_icon(TrayState::Error)))
                        .unwrap_or_else(|e| warn!("failed to set icon: {e}"));
                    tray.set_menu(Some(Box::new(menu::build_unavailable_menu())));
                    warn!("gateway unavailable: {reason}");
                }
            }
        }

        // Process menu clicks
        if let Ok(event) = menu_channel.try_recv() {
            match menu::parse_action(&event.id) {
                Some(Action::Quit) => {
                    info!("quit requested");
                    std::process::exit(0);
                }
                Some(Action::OpenUi) => {
                    launch_ui(None);
                }
                Some(action) => {
                    if action_tx.send(action).is_err() {
                        error!("async worker gone, exiting");
                        std::process::exit(1);
                    }
                }
                None => {}
            }
        }

        // Sleep briefly to avoid busy-spinning. The macOS run loop doesn't
        // provide a blocking wait that also drains our mpsc channels, so we
        // poll at ~60 Hz which is negligible CPU.
        std::thread::sleep(std::time::Duration::from_millis(16));
    }
}

async fn async_worker(
    port_override: Option<u16>,
    interval_secs: u64,
    poll_tx: mpsc::Sender<PollResult>,
    action_rx: mpsc::Receiver<Action>,
) {
    let interval = std::time::Duration::from_secs(interval_secs);

    // Initial discovery -- retry until gateway is reachable
    let mut client = match GatewayClient::discover(port_override).await {
        Ok(c) => c,
        Err(e) => {
            warn!("initial gateway discovery failed: {e}");
            let _ = poll_tx.send(PollResult::Unavailable(e.to_string()));
            // Keep trying
            loop {
                tokio::time::sleep(interval).await;
                match GatewayClient::discover(port_override).await {
                    Ok(c) => break c,
                    Err(e) => {
                        let _ = poll_tx.send(PollResult::Unavailable(e.to_string()));
                    }
                }
            }
        }
    };

    info!("gateway discovered at port {}", client.port());

    loop {
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

        // Drain pending actions before sleeping
        while let Ok(action) = action_rx.try_recv() {
            dispatch_action(&client, action).await;
        }

        tokio::time::sleep(interval).await;
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
