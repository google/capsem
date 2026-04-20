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

    /// PID of the capsem-service that spawned us. The tray is a companion
    /// process: it refuses to start without a live parent service and
    /// exits the moment that parent dies. See capsem-guard for details.
    #[arg(long)]
    parent_pid: Option<u32>,

    /// Path for the singleton lock file (overrides the default under
    /// CAPSEM_RUN_DIR). Used by tests that need an isolated namespace.
    #[arg(long)]
    lock_path: Option<std::path::PathBuf>,
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

    // Companion guards: (1) refuse to start without a live parent service,
    // (2) refuse to start if another tray already holds the singleton. Both
    // conditions are expected (stale launch, double-spawn race) and resolved
    // by exiting 0 -- not Err -- so the caller sees success.
    let lock_path = args.lock_path.clone().unwrap_or_else(tray_lock_path);
    match capsem_guard::install(args.parent_pid, &lock_path) {
        Ok(Some(_guards)) => {
            // `_guards` is bound here and held for the process's lifetime.
            // std::mem::forget would also work but keeping the binding
            // makes the ownership obvious.
            Box::leak(Box::new(_guards));
        }
        Ok(None) => {
            info!(
                lock = %lock_path.display(),
                "another capsem-tray is already running; exiting 0"
            );
            return Ok(());
        }
        Err(e) => {
            // No parent or parent dead: the tray was launched standalone or
            // re-parented to init. Not an error -- just not something we
            // should keep running for.
            info!(error = %e, "tray refusing to run without a live capsem-service; exiting 0");
            return Ok(());
        }
    }

    // Headless mode for tests: the companion-lifecycle suite exercises
    // parent-watch + singleton by spawning real tray binaries. Those runs
    // don't need a menu-bar icon, and creating real NSStatusItems on every
    // test flashes the user's menu bar. Gate the UI behind an env var;
    // when set, the tray still holds the guard/lock and idles until the
    // parent dies, which is what the tests observe.
    if std::env::var_os("CAPSEM_TRAY_HEADLESS").is_some() {
        info!("tray started in headless mode (no menu bar icon)");
        loop {
            std::thread::sleep(std::time::Duration::from_secs(60));
        }
    }

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
    launch_capsem_app(vm_id, None);
}

/// Launch the UI with a VM deep-link plus an action hint (e.g. "save", "fork").
/// The UI opens the VM tab and dispatches the action (opens the relevant modal)
/// -- actions requiring a user-supplied name belong in the UI, not the tray.
fn launch_ui_action(vm_id: &str, action: &str) {
    launch_capsem_app(Some(vm_id), Some(action));
}

/// Platform-specific install candidates for the Capsem.app binary.
///
/// Kept separate from `build_launch_invocation` so the construction logic
/// stays pure and unit-testable -- filesystem probes happen here.
fn find_capsem_app_binary() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "macos")]
    let candidates: [std::path::PathBuf; 2] = [
        std::path::PathBuf::from("/Applications/Capsem.app/Contents/MacOS/capsem-app"),
        std::env::var("HOME")
            .map(|h| {
                std::path::PathBuf::from(h)
                    .join("Applications/Capsem.app/Contents/MacOS/capsem-app")
            })
            .unwrap_or_default(),
    ];
    #[cfg(target_os = "linux")]
    let candidates: [std::path::PathBuf; 2] = [
        std::path::PathBuf::from("/usr/bin/capsem-app"),
        std::path::PathBuf::from("/usr/local/bin/capsem-app"),
    ];
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let candidates: [std::path::PathBuf; 0] = [];

    candidates.into_iter().find(|p| p.exists())
}

/// Build the (program, args) pair for launching the Capsem desktop app.
///
/// Pure, unit-tested. Exists so we can verify deep-link construction
/// (--connect / --action forwarding, `open --args` fallback) without
/// actually executing a binary.
///
/// On macOS, `open -a Capsem --args X` only delivers X when the app is
/// NOT already running. Invoking the binary path directly always spawns
/// a process; tauri-plugin-single-instance then forwards argv to the
/// running instance via its IPC socket.
fn build_launch_invocation(
    binary: Option<&std::path::Path>,
    vm_id: Option<&str>,
    action: Option<&str>,
) -> (std::ffi::OsString, Vec<std::ffi::OsString>) {
    let mut deep_link: Vec<std::ffi::OsString> = Vec::new();
    if let Some(id) = vm_id {
        deep_link.push("--connect".into());
        deep_link.push(id.into());
    }
    if let Some(a) = action {
        deep_link.push("--action".into());
        deep_link.push(a.into());
    }

    match binary {
        Some(path) => (path.as_os_str().to_owned(), deep_link),
        None => {
            let mut args: Vec<std::ffi::OsString> = vec!["-a".into(), "Capsem".into()];
            if !deep_link.is_empty() {
                args.push("--args".into());
                args.extend(deep_link);
            }
            ("open".into(), args)
        }
    }
}

/// Spawn the Capsem desktop app on a dedicated OS thread.
///
/// Two reasons this is `std::thread::spawn`, not `tokio::spawn_blocking`:
///
/// 1. The tray's tokio runtime is `new_current_thread` (a single worker).
///    `std::process::Command::spawn` invokes `posix_spawn`/`fork+exec`,
///    which counts as blocking I/O per `/dev-rust-patterns`. Blocking
///    the single worker would freeze status polling and action dispatch.
/// 2. The reaper `wait()` below blocks for the Capsem.app lifetime
///    (minutes to hours). Tokio's docs explicitly warn that
///    `spawn_blocking` workers should not be held that long -- they come
///    from a bounded pool meant for short operations. A plain OS thread
///    is the right tool for a long-lived reaper.
///
/// The reaper is why we hold onto the `Child` instead of dropping it --
/// a dropped `std::process::Child` leaves a zombie until the tray itself
/// exits, which on a long-running companion process means zombies
/// accumulate for every user-initiated action.
fn launch_capsem_app(vm_id: Option<&str>, action: Option<&str>) {
    info!(?vm_id, ?action, "launching Capsem.app");

    let vm_id = vm_id.map(str::to_string);
    let action = action.map(str::to_string);

    std::thread::spawn(move || {
        let binary = find_capsem_app_binary();
        if binary.is_none() {
            warn!(
                "capsem-app binary not found in install locations; falling back to `open -a Capsem`"
            );
        }
        let (program, args) =
            build_launch_invocation(binary.as_deref(), vm_id.as_deref(), action.as_deref());

        let mut cmd = std::process::Command::new(&program);
        cmd.args(&args);
        match cmd.spawn() {
            Ok(mut child) => {
                info!(?vm_id, ?action, "Capsem.app launch dispatched");
                let _ = child.wait();
            }
            Err(e) => warn!("failed to launch UI: {e}"),
        }
    });
}

/// Default path for the tray singleton lockfile.
///
/// The macOS menu bar is a shared global resource: a second NSStatusItem
/// shows up as a duplicate icon. The tray must therefore be a SYSTEM-WIDE
/// singleton, not scoped to CAPSEM_RUN_DIR. Parallel test workers each have
/// their own service + gateway (scoped per run_dir), but only one tray icon
/// ever lives in the menu bar -- the first one that wins the global lock.
/// When its parent service exits, the tray exits, and the next service's
/// tray can acquire. Tests that need strict isolation override with
/// `--lock-path`.
fn tray_lock_path() -> std::path::PathBuf {
    capsem_core::paths::capsem_run_dir().join("tray.lock")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::path::PathBuf;

    fn os(s: &str) -> OsString {
        OsString::from(s)
    }

    #[test]
    fn direct_binary_no_vm_id_no_action() {
        let binary = PathBuf::from("/Applications/Capsem.app/Contents/MacOS/capsem-app");
        let (program, args) = build_launch_invocation(Some(&binary), None, None);
        assert_eq!(program, binary.as_os_str());
        assert!(args.is_empty(), "no deep-link args expected, got {args:?}");
    }

    #[test]
    fn direct_binary_connects_to_vm() {
        let binary = PathBuf::from("/Applications/Capsem.app/Contents/MacOS/capsem-app");
        let (program, args) = build_launch_invocation(Some(&binary), Some("vm-123"), None);
        assert_eq!(program, binary.as_os_str());
        assert_eq!(args, vec![os("--connect"), os("vm-123")]);
    }

    #[test]
    fn direct_binary_with_action_requires_vm_id() {
        let binary = PathBuf::from("/Applications/Capsem.app/Contents/MacOS/capsem-app");
        let (program, args) =
            build_launch_invocation(Some(&binary), Some("vm-42"), Some("save"));
        assert_eq!(program, binary.as_os_str());
        assert_eq!(
            args,
            vec![os("--connect"), os("vm-42"), os("--action"), os("save")]
        );
    }

    #[test]
    fn fallback_open_no_args_when_no_deep_link() {
        // Without vm_id/action, `open -a Capsem` is enough -- no `--args`.
        // Appending `--args` with nothing after it would still work but is
        // unnecessary noise.
        let (program, args) = build_launch_invocation(None, None, None);
        assert_eq!(program, os("open"));
        assert_eq!(args, vec![os("-a"), os("Capsem")]);
    }

    #[test]
    fn fallback_open_forwards_vm_id() {
        // Regression guard: the pre-refactor launch_ui added `--args` only
        // when vm_id was Some, and launch_ui_action always added `--args`.
        // Check both paths go through one helper with consistent behavior.
        let (program, args) = build_launch_invocation(None, Some("vm-9"), None);
        assert_eq!(program, os("open"));
        assert_eq!(
            args,
            vec![os("-a"), os("Capsem"), os("--args"), os("--connect"), os("vm-9")]
        );
    }

    #[test]
    fn fallback_open_forwards_vm_id_and_action() {
        let (program, args) = build_launch_invocation(None, Some("vm-9"), Some("fork"));
        assert_eq!(program, os("open"));
        assert_eq!(
            args,
            vec![
                os("-a"),
                os("Capsem"),
                os("--args"),
                os("--connect"),
                os("vm-9"),
                os("--action"),
                os("fork"),
            ]
        );
    }
}

