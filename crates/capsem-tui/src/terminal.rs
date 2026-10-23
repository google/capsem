use std::collections::BTreeMap;
use std::sync::Mutex;
use std::thread;

use capsem_sdk::models::stream::{
    self as protocol, ServerFrame, StreamChannel, StreamControl, StreamKind, StreamStatus,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use futures::{SinkExt, StreamExt};
use tokio::sync::mpsc as tokio_mpsc;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};

const MAX_TERMINAL_INPUTS_PER_SEND: usize = 128;

#[derive(Debug)]
pub struct TerminalBridge {
    commands: tokio_mpsc::UnboundedSender<TerminalCommand>,
    events: Mutex<tokio_mpsc::Receiver<TerminalEvent>>,
}

impl TerminalBridge {
    pub fn spawn(base_url: String) -> Self {
        let (command_tx, command_rx) = tokio_mpsc::unbounded_channel();
        let (event_tx, event_rx) = event_channel();
        thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build capsem-tui terminal runtime");
            runtime.block_on(run_terminal_manager(base_url, command_rx, event_tx));
        });
        Self {
            commands: command_tx,
            events: Mutex::new(event_rx),
        }
    }

    pub fn connect(&self, session_id: impl Into<String>, cols: u16, rows: u16) {
        let _ = self.commands.send(TerminalCommand::Connect {
            session_id: session_id.into(),
            cols,
            rows,
        });
    }

    pub fn input(&self, bytes: Vec<u8>) {
        let _ = self.commands.send(TerminalCommand::Input(bytes));
    }

    pub fn resize(&self, cols: u16, rows: u16) {
        let _ = self.commands.send(TerminalCommand::Resize { cols, rows });
    }

    pub fn focus(&self, cols: u16, rows: u16) {
        let _ = self.commands.send(TerminalCommand::Focus { cols, rows });
    }

    pub fn disconnect(&self) {
        let _ = self.commands.send(TerminalCommand::Disconnect);
    }

    pub fn drain_events(&self) -> Vec<TerminalEvent> {
        let mut events = Vec::new();
        let mut receiver = self.events.lock().unwrap();
        for _ in 0..protocol::MAX_TERMINAL_VIEW_UPDATES {
            let Ok(event) = receiver.try_recv() else { break };
            push_coalesced_event(&mut events, event);
        }
        drop(receiver);
        events
    }
}

impl Drop for TerminalBridge {
    fn drop(&mut self) {
        let _ = self.commands.send(TerminalCommand::Shutdown);
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum TerminalCommand {
    Connect { session_id: String, cols: u16, rows: u16 },
    Input(Vec<u8>),
    Resize { cols: u16, rows: u16 },
    Focus { cols: u16, rows: u16 },
    Disconnect,
    Shutdown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TerminalEvent {
    State {
        session_id: String,
        state: Box<protocol::TerminalState>,
    },
    Output {
        session_id: String,
        bytes: Vec<u8>,
    },
    Status {
        session_id: String,
        status: String,
    },
}

fn push_coalesced_event(events: &mut Vec<TerminalEvent>, event: TerminalEvent) {
    match (events.last_mut(), event) {
        (
            Some(TerminalEvent::Output {
                session_id: previous_id,
                bytes: previous_bytes,
            }),
            TerminalEvent::Output { session_id, bytes },
        ) if previous_id == &session_id => {
            previous_bytes.extend_from_slice(&bytes);
        }
        (_, event) => events.push(event),
    }
}

async fn run_terminal_manager(
    base_url: String,
    mut commands: tokio_mpsc::UnboundedReceiver<TerminalCommand>,
    events: tokio_mpsc::Sender<TerminalEvent>,
) {
    let mut active_session_id = String::new();
    let mut active_input: Option<tokio_mpsc::UnboundedSender<TerminalInput>> = None;
    let mut active_task: Option<tokio::task::JoinHandle<()>> = None;

    loop {
        let command = if let Some(task) = &mut active_task {
            tokio::select! {
                command = commands.recv() => command,
                result = task => {
                    let _ = result;
                    active_task = None;
                    active_input = None;
                    active_session_id.clear();
                    continue;
                }
            }
        } else {
            commands.recv().await
        };
        let Some(command) = command else {
            if let Some(task) = active_task.take() {
                task.abort();
            }
            break;
        };
        match command {
            TerminalCommand::Connect { session_id, cols, rows } => {
                if session_id == active_session_id && active_input.is_some() {
                    let resize_sent = active_input
                        .as_ref()
                        .is_some_and(|input| input.send(TerminalInput::Focus { cols, rows }).is_ok());
                    if resize_sent {
                        continue;
                    }
                    if let Some(task) = active_task.take() {
                        task.abort();
                    }
                }
                if let Some(task) = active_task.take() {
                    task.abort();
                }
                let (input_tx, input_rx) = tokio_mpsc::unbounded_channel();
                active_input = Some(input_tx.clone());
                active_session_id.clone_from(&session_id);
                let task_base_url = base_url.clone();
                let task_events = events.clone();
                active_task = Some(tokio::spawn(async move {
                    run_terminal_connection(task_base_url, session_id, cols, rows, input_rx, task_events).await;
                }));
            }
            TerminalCommand::Input(bytes) => {
                if let Some(input) = &active_input {
                    let _ = input.send(TerminalInput::Bytes(bytes));
                }
            }
            TerminalCommand::Resize { cols, rows } => {
                if let Some(input) = &active_input {
                    let _ = input.send(TerminalInput::Resize { cols, rows });
                }
            }
            TerminalCommand::Focus { cols, rows } => {
                if let Some(input) = &active_input {
                    let _ = input.send(TerminalInput::Focus { cols, rows });
                }
            }
            TerminalCommand::Disconnect => {
                if let Some(task) = active_task.take() {
                    task.abort();
                }
                active_input = None;
                active_session_id.clear();
            }
            TerminalCommand::Shutdown => {
                if let Some(task) = active_task.take() {
                    task.abort();
                }
                break;
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum TerminalInput {
    Bytes(Vec<u8>),
    Focus { cols: u16, rows: u16 },
    Resize { cols: u16, rows: u16 },
}

async fn run_terminal_connection(
    base_url: String,
    session_id: String,
    cols: u16,
    rows: u16,
    mut input_rx: tokio_mpsc::UnboundedReceiver<TerminalInput>,
    events: tokio_mpsc::Sender<TerminalEvent>,
) {
    let token =
        match crate::gateway_provider::load_token(&base_url, &crate::gateway_provider::run_dir().join("gateway.token"))
            .await
        {
            Ok(token) => token,
            Err(error) => {
                send_status(&events, &session_id, format!("token failed: {error:#}")).await;
                return;
            }
        };
    let mut request = match stream_ws_url(&base_url, &session_id, &token).into_client_request() {
        Ok(request) => request,
        Err(error) => {
            send_status(&events, &session_id, format!("connect failed: {error:#}")).await;
            return;
        }
    };
    request
        .headers_mut()
        .insert("sec-websocket-protocol", http_header(protocol::STREAM_SUBPROTOCOL));
    let (socket, _) = match connect_async(request).await {
        Ok(socket) => socket,
        Err(error) => {
            send_status(&events, &session_id, format!("connect failed: {error:#}")).await;
            return;
        }
    };
    let (mut write, mut read) = socket.split();
    let frames = match stream_start_frames(cols, rows) {
        Ok(frames) => frames,
        Err(error) => {
            send_status(&events, &session_id, format!("protocol error: {error}")).await;
            return;
        }
    };
    for frame in frames {
        if let Err(error) = write.send(Message::Binary(frame.into())).await {
            send_status(&events, &session_id, format!("send failed: {error:#}")).await;
            return;
        }
    }

    loop {
        tokio::select! {
            input = input_rx.recv() => {
                let Some(input) = input else {
                    break;
                };
                for input in coalesced_terminal_inputs(input, &mut input_rx) {
                    let frames = match stream_input_frames(input) {
                        Ok(frames) => frames,
                        Err(error) => {
                            send_status(&events, &session_id, format!("protocol error: {error}")).await;
                            return;
                        }
                    };
                    for frame in frames {
                        if let Err(error) = write.send(Message::Binary(frame.into())).await {
                            send_status(&events, &session_id, format!("send failed: {error:#}")).await;
                            return;
                        }
                    }
                }
            }
            message = read.next() => {
                match message {
                    Some(Ok(Message::Binary(bytes))) => match stream_server_event(&bytes) {
                        Some(StreamEvent::Output(bytes)) => {
                            let _ = events.send(TerminalEvent::Output {
                                session_id: session_id.clone(),
                                bytes,
                            }).await;
                        }
                        Some(StreamEvent::State(state)) => {
                            let _ = events.send(TerminalEvent::State { session_id: session_id.clone(), state }).await;
                        }
                        Some(StreamEvent::Status(status)) => send_status(&events, &session_id, status).await,
                        None => {}
                    },
                    Some(Ok(Message::Close(_))) | None => {
                        send_status(&events, &session_id, "disconnected").await;
                        break;
                    }
                    Some(Ok(Message::Text(_) | Message::Ping(_) | Message::Pong(_) | Message::Frame(_))) => {}
                    Some(Err(error)) => {
                        send_status(&events, &session_id, format!("read failed: {error:#}")).await;
                        break;
                    }
                }
            }
        }
    }
}

fn http_header(value: &'static str) -> tokio_tungstenite::tungstenite::http::HeaderValue {
    tokio_tungstenite::tungstenite::http::HeaderValue::from_static(value)
}

/// What a terminal stream opens with: `start`, then the window size.
fn stream_start_frames(cols: u16, rows: u16) -> Result<Vec<Vec<u8>>, protocol::StreamFrameError> {
    let mut frames = vec![protocol::encode_control(&StreamControl::Start {
        kind: StreamKind::Terminal,
        command: None,
    })?];
    if cols > 0 && rows > 0 {
        frames.push(protocol::encode_control(&StreamControl::Focus { cols, rows })?);
    }
    Ok(frames)
}

/// One client frame for terminal input; a zero-sized window is not a resize.
fn stream_input_frames(input: TerminalInput) -> Result<Vec<Vec<u8>>, protocol::StreamFrameError> {
    match input {
        TerminalInput::Bytes(bytes) => bytes
            .chunks(protocol::MAX_STREAM_DATA_BYTES)
            .map(|chunk| protocol::encode_data(StreamChannel::Stdin, chunk))
            .collect(),
        TerminalInput::Resize { cols, rows } if cols > 0 && rows > 0 => {
            Ok(vec![protocol::encode_control(&StreamControl::Resize { cols, rows })?])
        }
        TerminalInput::Focus { cols, rows } if cols > 0 && rows > 0 => {
            Ok(vec![protocol::encode_control(&StreamControl::Focus { cols, rows })?])
        }
        TerminalInput::Resize { .. } | TerminalInput::Focus { .. } => Ok(Vec::new()),
    }
}

#[derive(Debug, PartialEq, Eq)]
enum StreamEvent {
    State(Box<protocol::TerminalState>),
    Output(Vec<u8>),
    Status(String),
}

fn stream_server_event(frame: &[u8]) -> Option<StreamEvent> {
    match protocol::decode_server_frame(frame) {
        Ok(ServerFrame::Stdout(bytes) | ServerFrame::Stderr(bytes)) => Some(StreamEvent::Output(bytes.to_vec())),
        Ok(ServerFrame::Status(StreamStatus::Terminal { state })) => Some(StreamEvent::State(state)),
        Ok(ServerFrame::Status(StreamStatus::Started)) => Some(StreamEvent::Status("connected".into())),
        Ok(ServerFrame::Status(StreamStatus::Error { message })) => Some(StreamEvent::Status(message)),
        Ok(ServerFrame::Status(StreamStatus::Exit { code, .. })) => Some(StreamEvent::Status(format!("exited {code}"))),
        Err(error) => Some(StreamEvent::Status(format!("protocol error: {error}"))),
    }
}

fn coalesced_terminal_inputs(
    first: TerminalInput,
    input_rx: &mut tokio_mpsc::UnboundedReceiver<TerminalInput>,
) -> Vec<TerminalInput> {
    let mut inputs = vec![first];
    let mut consumed = 1usize;
    while consumed < MAX_TERMINAL_INPUTS_PER_SEND {
        let Ok(input) = input_rx.try_recv() else {
            break;
        };
        consumed += 1;
        match (inputs.last_mut(), input) {
            (Some(TerminalInput::Bytes(previous)), TerminalInput::Bytes(mut bytes)) => {
                previous.append(&mut bytes);
            }
            (_, input) => inputs.push(input),
        }
    }
    inputs
}

fn stream_ws_url(base_url: &str, session_id: &str, token: &str) -> String {
    let base = base_url.trim_end_matches('/');
    let ws_base = if let Some(rest) = base.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = base.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        base.to_string()
    };
    format!(
        "{ws_base}/vms/{}/stream?token={}",
        url_encode_component(session_id),
        url_encode_component(token)
    )
}

fn url_encode_component(value: &str) -> String {
    value
        .bytes()
        .flat_map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                vec![byte as char]
            }
            _ => format!("%{byte:02X}").chars().collect(),
        })
        .collect()
}

async fn send_status(events: &tokio_mpsc::Sender<TerminalEvent>, session_id: &str, status: impl Into<String>) {
    let _ = events
        .send(TerminalEvent::Status {
            session_id: session_id.to_string(),
            status: status.into(),
        })
        .await;
}

pub struct TerminalSurface {
    buffers: BTreeMap<String, TerminalBuffer>,
}

impl TerminalSurface {
    pub fn new() -> Self {
        Self {
            buffers: BTreeMap::new(),
        }
    }

    pub fn apply(&mut self, event: TerminalEvent) {
        match event {
            TerminalEvent::State { session_id, state } => {
                let buffer = self.buffer_mut(&session_id);
                if state.reset {
                    buffer.parser = vt100::Parser::new(state.rows, state.cols, 0);
                } else {
                    buffer.parser.screen_mut().set_size(state.rows, state.cols);
                }
                buffer.state = Some(state);
            }
            TerminalEvent::Output { session_id, bytes } => {
                self.buffer_mut(&session_id).append(&bytes);
            }
            TerminalEvent::Status { session_id, status } => {
                self.buffer_mut(&session_id).status = Some(status);
            }
        }
    }

    pub fn lines_for(&self, session_id: &str, height: usize) -> Vec<String> {
        self.styled_lines_for(session_id, height)
            .into_iter()
            .map(|line| line.plain_text())
            .collect()
    }

    pub fn styled_lines_for(&self, session_id: &str, height: usize) -> Vec<TerminalLine> {
        self.buffers
            .get(session_id)
            .map(|buffer| buffer.visible_lines(height))
            .unwrap_or_default()
    }

    pub fn cursor_for(&self, session_id: &str, width: u16, height: u16) -> Option<(u16, u16)> {
        let state = self.buffers.get(session_id)?.state.as_ref()?;
        if state.cursor.hidden || width == 0 || height == 0 {
            return None;
        }
        let row = state.cursor.row.checked_sub(state.rows.saturating_sub(height))?;
        let col = state.cursor.col.min(state.cols - 1);
        (row < height && col < width).then_some((col, row))
    }

    pub fn input_for(&self, session_id: &str, key: KeyEvent) -> Option<Vec<u8>> {
        let application = self
            .buffers
            .get(session_id)
            .and_then(|b| b.state.as_ref())
            .is_some_and(|state| state.modes.application_cursor);
        key_bytes(key, application)
    }

    pub fn paste_for(&self, session_id: &str, text: &str) -> Vec<u8> {
        let bracketed = self
            .buffers
            .get(session_id)
            .and_then(|b| b.state.as_ref())
            .is_some_and(|state| state.modes.bracketed_paste);
        if bracketed {
            format!("\x1b[200~{text}\x1b[201~").into_bytes()
        } else {
            text.as_bytes().to_vec()
        }
    }

    pub fn status_for(&self, session_id: &str) -> Option<&str> {
        self.buffers.get(session_id).and_then(|buffer| buffer.status.as_deref())
    }

    fn buffer_mut(&mut self, session_id: &str) -> &mut TerminalBuffer {
        self.buffers.entry(session_id.to_string()).or_default()
    }
}

impl Default for TerminalSurface {
    fn default() -> Self {
        Self::new()
    }
}

struct TerminalBuffer {
    parser: vt100::Parser,
    state: Option<Box<protocol::TerminalState>>,
    status: Option<String>,
}

impl TerminalBuffer {
    fn append(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
    }

    fn visible_lines(&self, height: usize) -> Vec<TerminalLine> {
        let screen = self.parser.screen();
        let (rows, cols) = screen.size();
        let start_row = usize::from(rows).saturating_sub(height);
        (start_row..usize::from(rows))
            .map(|row| line_from_screen_row(screen, row as u16, cols))
            .collect()
    }
}

impl Default for TerminalBuffer {
    fn default() -> Self {
        Self {
            parser: vt100::Parser::new(24, 80, 0),
            state: None,
            status: None,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TerminalLine {
    spans: Vec<TerminalSpan>,
}

impl TerminalLine {
    pub fn spans(&self) -> &[TerminalSpan] {
        &self.spans
    }

    pub fn plain_text(&self) -> String {
        self.spans
            .iter()
            .map(|span| span.text.as_str())
            .collect::<String>()
            .trim_end()
            .to_string()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalSpan {
    pub text: String,
    pub style: TerminalStyle,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TerminalStyle {
    pub fg: TerminalColor,
    pub bg: TerminalColor,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub inverse: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TerminalColor {
    #[default]
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

fn line_from_screen_row(screen: &vt100::Screen, row: u16, cols: u16) -> TerminalLine {
    let mut line = TerminalLine::default();
    for col in 0..cols {
        let Some(cell) = screen.cell(row, col) else {
            continue;
        };
        if cell.is_wide_continuation() {
            continue;
        }
        let text = if cell.has_contents() { cell.contents() } else { " " };
        push_screen_text(&mut line, text, style_from_cell(cell));
    }
    trim_terminal_line(&mut line);
    line
}

fn push_screen_text(line: &mut TerminalLine, text: &str, style: TerminalStyle) {
    if let Some(span) = line.spans.last_mut().filter(|span| span.style == style) {
        span.text.push_str(text);
        return;
    }
    line.spans.push(TerminalSpan {
        text: text.to_string(),
        style,
    });
}

fn trim_terminal_line(line: &mut TerminalLine) {
    while let Some(span) = line.spans.last_mut() {
        let trimmed = span.text.trim_end_matches(' ');
        if trimmed.len() == span.text.len() {
            break;
        }
        span.text.truncate(trimmed.len());
        if !span.text.is_empty() {
            break;
        }
        line.spans.pop();
    }
}

fn style_from_cell(cell: &vt100::Cell) -> TerminalStyle {
    TerminalStyle {
        fg: color_from_vt100(cell.fgcolor()),
        bg: color_from_vt100(cell.bgcolor()),
        bold: cell.bold(),
        dim: cell.dim(),
        italic: cell.italic(),
        underline: cell.underline(),
        inverse: cell.inverse(),
    }
}

fn color_from_vt100(color: vt100::Color) -> TerminalColor {
    match color {
        vt100::Color::Default => TerminalColor::Default,
        vt100::Color::Idx(index) => TerminalColor::Indexed(index),
        vt100::Color::Rgb(red, green, blue) => TerminalColor::Rgb(red, green, blue),
    }
}

pub fn key_to_terminal_bytes(key: KeyEvent) -> Option<Vec<u8>> {
    key_bytes(key, false)
}

fn key_bytes(key: KeyEvent, application_cursor: bool) -> Option<Vec<u8>> {
    if key.modifiers.intersects(KeyModifiers::SUPER) {
        return None;
    }
    let modifier = 1
        + u8::from(key.modifiers.contains(KeyModifiers::SHIFT))
        + 2 * u8::from(key.modifiers.contains(KeyModifiers::ALT))
        + 4 * u8::from(key.modifiers.contains(KeyModifiers::CONTROL));
    let final_byte = match key.code {
        KeyCode::Up => Some('A'),
        KeyCode::Down => Some('B'),
        KeyCode::Right => Some('C'),
        KeyCode::Left => Some('D'),
        KeyCode::Home => Some('H'),
        KeyCode::End => Some('F'),
        KeyCode::F(n @ 1..=4) => Some(char::from(b'P' + n - 1)),
        _ => None,
    };
    if let Some(final_byte) = final_byte {
        return Some(
            if modifier > 1 {
                format!("\x1b[1;{modifier}{final_byte}")
            } else if application_cursor || matches!(key.code, KeyCode::F(_)) {
                format!("\x1bO{final_byte}")
            } else {
                format!("\x1b[{final_byte}")
            }
            .into_bytes(),
        );
    }
    let number = match key.code {
        KeyCode::Insert => Some(2),
        KeyCode::Delete => Some(3),
        KeyCode::PageUp => Some(5),
        KeyCode::PageDown => Some(6),
        KeyCode::F(n @ 5..=12) => Some([15, 17, 18, 19, 20, 21, 23, 24][usize::from(n - 5)]),
        _ => None,
    };
    if let Some(number) = number {
        return Some(
            if modifier > 1 {
                format!("\x1b[{number};{modifier}~")
            } else {
                format!("\x1b[{number}~")
            }
            .into_bytes(),
        );
    }
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        return control_key_bytes(key.code);
    }
    let mut bytes = Vec::new();
    if key.modifiers.contains(KeyModifiers::ALT) {
        bytes.push(0x1b);
    }
    match key.code {
        KeyCode::Backspace => bytes.push(0x7f),
        KeyCode::Enter => bytes.push(b'\r'),
        KeyCode::Tab => bytes.push(b'\t'),
        KeyCode::BackTab => bytes.extend_from_slice(b"\x1b[Z"),
        KeyCode::Esc => bytes.push(0x1b),
        KeyCode::Char(ch) => bytes.extend(ch.to_string().as_bytes()),
        _ => return None,
    }
    Some(bytes)
}

fn control_key_bytes(code: KeyCode) -> Option<Vec<u8>> {
    match code {
        KeyCode::Char(ch) if ch.is_ascii_alphabetic() => {
            let value = ch.to_ascii_lowercase() as u8 - b'a' + 1;
            Some(vec![value])
        }
        KeyCode::Char('[') | KeyCode::Esc => Some(vec![0x1b]),
        KeyCode::Char(']') => Some(vec![0x1d]),
        KeyCode::Char('\\') => Some(vec![0x1c]),
        KeyCode::Char('^') => Some(vec![0x1e]),
        KeyCode::Char('_') => Some(vec![0x1f]),
        KeyCode::Backspace => Some(vec![0x08]),
        _ => None,
    }
}

#[cfg(test)]
mod tests;

fn event_channel() -> (tokio_mpsc::Sender<TerminalEvent>, tokio_mpsc::Receiver<TerminalEvent>) {
    tokio_mpsc::channel(protocol::MAX_TERMINAL_VIEW_UPDATES)
}
