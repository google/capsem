use std::io;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use capsem_tui::app::{App, AppAction, ControlAction};
use capsem_tui::fixture::{offline_state, FixtureProvider};
use capsem_tui::gateway_provider::{ActionOutcome, GatewayProvider};
use capsem_tui::model::{AppState, ServiceStatus, SessionLifecycle};
use capsem_tui::provider::StateProvider;
use capsem_tui::terminal::{key_to_terminal_bytes, TerminalBridge, TerminalEvent, TerminalSurface};
use capsem_tui::ui::{render_app, render_app_snapshot, render_app_svg_snapshot};
use clap::Parser;
use crossterm::event::{self, Event, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

const UI_TICK_INTERVAL: Duration = Duration::from_millis(16);

#[derive(Parser)]
#[command(author, version, about = "Capsem terminal control UI")]
struct Cli {
    /// Print a deterministic text rendering instead of opening the terminal UI.
    #[arg(long)]
    snapshot: bool,

    /// Print a deterministic SVG rendering instead of opening the terminal UI.
    #[arg(long)]
    snapshot_svg: bool,

    /// Use the built-in two-session fixture instead of the installed Capsem gateway.
    #[arg(long)]
    fixture: bool,

    /// Capsem gateway base URL. Defaults to installed runtime files, then 127.0.0.1:19222.
    #[arg(long)]
    gateway_url: Option<String>,

    /// Live gateway refresh interval in milliseconds.
    #[arg(long, default_value_t = 1_000)]
    refresh_ms: u64,

    /// Start focused on a specific session id or title.
    #[arg(long)]
    session: Option<String>,

    /// Snapshot width.
    #[arg(long, default_value_t = 100)]
    width: u16,

    /// Snapshot height.
    #[arg(long, default_value_t = 24)]
    height: u16,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let state = load_state(&cli)?;
    let app = app_from_state(state, cli.session.as_deref())?;

    if cli.snapshot_svg {
        println!("{}", render_app_svg_snapshot(&app, cli.width, cli.height)?);
        return Ok(());
    }

    if cli.snapshot {
        println!("{}", render_app_snapshot(&app, cli.width, cli.height)?);
        return Ok(());
    }

    let live_provider = live_provider(&cli);
    let terminal_bridge = live_provider
        .as_ref()
        .map(|provider| TerminalBridge::spawn(provider.base_url().to_string()));

    run_interactive(app, live_provider, terminal_bridge, cli.refresh_interval())
}

fn load_state(cli: &Cli) -> Result<AppState> {
    if cli.fixture {
        return FixtureProvider.load();
    }

    let base_url = cli
        .gateway_url
        .clone()
        .unwrap_or_else(GatewayProvider::default_base_url);
    match GatewayProvider::new(base_url.clone()).load() {
        Ok(state) => Ok(state),
        Err(_) if cli.gateway_url.is_none() => Ok(offline_state()),
        Err(error) => {
            Err(error).with_context(|| format!("load capsem gateway state from {base_url}"))
        }
    }
}

fn app_from_state(state: AppState, session: Option<&str>) -> Result<App> {
    let mut app = App::new(state);
    if let Some(session) = session {
        if !app.select_session_by_id(session) {
            anyhow::bail!("session not found in TUI state: {session}");
        }
    }
    Ok(app)
}

fn live_provider(cli: &Cli) -> Option<GatewayProvider> {
    if cli.fixture {
        return None;
    }
    Some(GatewayProvider::new(
        cli.gateway_url
            .clone()
            .unwrap_or_else(GatewayProvider::default_base_url),
    ))
}

impl Cli {
    fn refresh_interval(&self) -> Duration {
        Duration::from_millis(self.refresh_ms.max(100))
    }
}

fn run_interactive(
    mut app: App,
    live_provider: Option<GatewayProvider>,
    terminal_bridge: Option<TerminalBridge>,
    refresh_interval: Duration,
) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_loop(
        &mut terminal,
        &mut app,
        live_provider.clone(),
        terminal_bridge,
        live_provider.map(ControlBridge::spawn),
        refresh_interval,
    );

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    live_provider: Option<GatewayProvider>,
    terminal_bridge: Option<TerminalBridge>,
    control_bridge: Option<ControlBridge>,
    refresh_interval: Duration,
) -> Result<()> {
    let mut last_refresh = Instant::now();
    let mut surface = TerminalSurface::new();
    let mut connected_terminal = None;
    let mut needs_draw = true;
    let input_events = spawn_input_reader();
    loop {
        if let Some(bridge) = &control_bridge {
            let mut should_refresh = false;
            let mut focus_after_refresh = None;
            for event in bridge.drain_events() {
                needs_draw = true;
                match event {
                    ControlEvent::Started(label) => {
                        app.set_control_message(format!("{label}..."));
                    }
                    ControlEvent::Finished(Ok(outcome)) => {
                        app.set_control_message(outcome.message);
                        focus_after_refresh = outcome.focus_session;
                        should_refresh = true;
                    }
                    ControlEvent::Finished(Err(error)) => {
                        app.set_control_message(error);
                        should_refresh = true;
                    }
                }
            }
            if should_refresh {
                needs_draw |= refresh_state(app, live_provider.as_ref());
                if let Some(session_id) = focus_after_refresh {
                    needs_draw |= app.select_session_by_id(&session_id);
                }
            }
        }
        if let Some(bridge) = &terminal_bridge {
            let events = bridge.drain_events();
            if !events.is_empty() {
                needs_draw = true;
            }
            for event in events {
                if terminal_event_closes_connection(&event, connected_terminal.as_ref()) {
                    bridge.disconnect();
                    connected_terminal = None;
                }
                surface.apply(event);
            }
            let size = terminal.size()?;
            let active_id = app.state().active_session_id.clone();
            let surface_rows = terminal_rows(size.height);
            if !active_id.is_empty() {
                surface.resize(&active_id, size.width.max(1), surface_rows);
            }
            needs_draw |= sync_terminal_connection(
                app,
                bridge,
                &mut connected_terminal,
                size.width.max(1),
                surface_rows,
            );
        }
        if last_refresh.elapsed() >= refresh_interval {
            needs_draw |= refresh_state(app, live_provider.as_ref());
            last_refresh = Instant::now();
        }
        if needs_draw {
            terminal.draw(|frame| render_app(frame, app, Some(&surface)))?;
            needs_draw = false;
        }
        match input_events.recv_timeout(UI_TICK_INTERVAL) {
            Ok(Ok(event)) => {
                if handle_terminal_event(
                    event,
                    app,
                    terminal_bridge.as_ref(),
                    control_bridge.as_ref(),
                )? {
                    break;
                }
                needs_draw = true;
            }
            Ok(Err(error)) => return Err(error).context("read terminal input event"),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    Ok(())
}

fn spawn_input_reader() -> mpsc::Receiver<io::Result<Event>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || loop {
        if tx.send(event::read()).is_err() {
            break;
        }
    });
    rx
}

fn handle_terminal_event(
    event: Event,
    app: &mut App,
    terminal_bridge: Option<&TerminalBridge>,
    control_bridge: Option<&ControlBridge>,
) -> Result<bool> {
    match event {
        Event::Key(key) if matches!(key.kind, KeyEventKind::Release) => {}
        Event::Key(key) => match app.handle_key(key) {
            AppAction::Exit => return Ok(true),
            AppAction::Consumed => {}
            AppAction::Invoke(action) => {
                if let Some(bridge) = control_bridge {
                    bridge.invoke(action);
                } else {
                    app.set_control_message("fixture action ignored");
                }
            }
            AppAction::Forward => {
                if let (Some(bridge), Some(bytes)) = (terminal_bridge, key_to_terminal_bytes(key)) {
                    bridge.input(bytes);
                }
            }
        },
        Event::Resize(width, height) => {
            if let Some(bridge) = terminal_bridge {
                bridge.resize(width.max(1), terminal_rows(height));
            }
        }
        _ => {}
    }
    Ok(false)
}

struct ControlBridge {
    commands: mpsc::Sender<ControlAction>,
    events: mpsc::Receiver<ControlEvent>,
}

impl ControlBridge {
    fn spawn(provider: GatewayProvider) -> Self {
        let (command_tx, command_rx) = mpsc::channel::<ControlAction>();
        let (event_tx, event_rx) = mpsc::channel::<ControlEvent>();
        thread::spawn(move || {
            while let Ok(action) = command_rx.recv() {
                let label = action.label().to_string();
                let _ = event_tx.send(ControlEvent::Started(label));
                let result = provider
                    .invoke(&action)
                    .map_err(|error| format!("{} failed: {error}", action.label()));
                let _ = event_tx.send(ControlEvent::Finished(result));
            }
        });
        Self {
            commands: command_tx,
            events: event_rx,
        }
    }

    fn invoke(&self, action: ControlAction) {
        let _ = self.commands.send(action);
    }

    fn drain_events(&self) -> Vec<ControlEvent> {
        let mut events = Vec::new();
        while let Ok(event) = self.events.try_recv() {
            events.push(event);
        }
        events
    }
}

enum ControlEvent {
    Started(String),
    Finished(std::result::Result<ActionOutcome, String>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ConnectedTerminal {
    session_id: String,
    cols: u16,
    rows: u16,
}

fn sync_terminal_connection(
    app: &App,
    bridge: &TerminalBridge,
    connected: &mut Option<ConnectedTerminal>,
    cols: u16,
    rows: u16,
) -> bool {
    let active_id = match active_terminal_session_id(app.state()) {
        Some(active_id) => active_id,
        None => {
            if connected.take().is_some() {
                bridge.disconnect();
                return true;
            }
            return false;
        }
    };
    let cols = cols.max(1);
    let rows = rows.max(1);
    match connected {
        Some(current) if current.session_id == active_id => {
            if current.cols == cols && current.rows == rows {
                return false;
            }
            bridge.resize(cols, rows);
            current.cols = cols;
            current.rows = rows;
            true
        }
        _ => {
            bridge.connect(active_id.to_string(), cols, rows);
            *connected = Some(ConnectedTerminal {
                session_id: active_id.to_string(),
                cols,
                rows,
            });
            true
        }
    }
}

fn active_terminal_session_id(state: &AppState) -> Option<&str> {
    let session = state.active_session()?;
    if matches!(
        session.lifecycle,
        SessionLifecycle::Working | SessionLifecycle::WaitingForInput
    ) {
        Some(session.id.as_str())
    } else {
        None
    }
}

fn terminal_event_closes_connection(
    event: &TerminalEvent,
    connected: Option<&ConnectedTerminal>,
) -> bool {
    let Some(connected) = connected else {
        return false;
    };
    let TerminalEvent::Status { session_id, status } = event else {
        return false;
    };
    session_id == &connected.session_id && terminal_status_is_closed(status)
}

fn terminal_status_is_closed(status: &str) -> bool {
    status == "disconnected"
        || status.starts_with("token failed:")
        || status.starts_with("connect failed:")
        || status.starts_with("send failed:")
        || status.starts_with("read failed:")
}

fn refresh_state(app: &mut App, provider: Option<&GatewayProvider>) -> bool {
    let Some(provider) = provider else {
        return false;
    };
    match provider.load() {
        Ok(state) => {
            app.replace_state(state);
            true
        }
        Err(_) => {
            let mut state = app.state().clone();
            state.service.status = ServiceStatus::Offline;
            state.service.latency = Duration::ZERO;
            state.service.reconnect_attempt = Some(
                state
                    .service
                    .reconnect_attempt
                    .unwrap_or_default()
                    .saturating_add(1),
            );
            app.replace_state(state);
            true
        }
    }
}

fn terminal_rows(height: u16) -> u16 {
    height.saturating_sub(1).max(1)
}

#[cfg(test)]
mod main_tests;
