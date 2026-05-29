use std::collections::BTreeMap;
use std::sync::mpsc;
use std::thread;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use futures::{SinkExt, StreamExt};
use tokio::sync::mpsc as tokio_mpsc;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

const MAX_SCROLLBACK_LINES: usize = 2_000;

#[derive(Debug)]
pub struct TerminalBridge {
    commands: tokio_mpsc::UnboundedSender<TerminalCommand>,
    events: mpsc::Receiver<TerminalEvent>,
}

impl TerminalBridge {
    pub fn spawn(base_url: String) -> Self {
        let (command_tx, command_rx) = tokio_mpsc::unbounded_channel();
        let (event_tx, event_rx) = mpsc::channel();
        thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build capsem-tui terminal runtime");
            runtime.block_on(run_terminal_manager(base_url, command_rx, event_tx));
        });
        Self {
            commands: command_tx,
            events: event_rx,
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

    pub fn drain_events(&self) -> Vec<TerminalEvent> {
        let mut events = Vec::new();
        while let Ok(event) = self.events.try_recv() {
            events.push(event);
        }
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
    Connect {
        session_id: String,
        cols: u16,
        rows: u16,
    },
    Input(Vec<u8>),
    Resize {
        cols: u16,
        rows: u16,
    },
    Shutdown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TerminalEvent {
    Output { session_id: String, bytes: Vec<u8> },
    Status { session_id: String, status: String },
}

async fn run_terminal_manager(
    base_url: String,
    mut commands: tokio_mpsc::UnboundedReceiver<TerminalCommand>,
    events: mpsc::Sender<TerminalEvent>,
) {
    let mut active_session_id = String::new();
    let mut active_input: Option<tokio_mpsc::UnboundedSender<TerminalInput>> = None;
    let mut active_task: Option<tokio::task::JoinHandle<()>> = None;

    while let Some(command) = commands.recv().await {
        match command {
            TerminalCommand::Connect {
                session_id,
                cols,
                rows,
            } => {
                if session_id == active_session_id && active_input.is_some() {
                    if let Some(input) = &active_input {
                        let _ = input.send(TerminalInput::Resize { cols, rows });
                    }
                    continue;
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
                    run_terminal_connection(
                        task_base_url,
                        session_id,
                        cols,
                        rows,
                        input_rx,
                        task_events,
                    )
                    .await;
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
    Resize { cols: u16, rows: u16 },
}

async fn run_terminal_connection(
    base_url: String,
    session_id: String,
    cols: u16,
    rows: u16,
    mut input_rx: tokio_mpsc::UnboundedReceiver<TerminalInput>,
    events: mpsc::Sender<TerminalEvent>,
) {
    let client = reqwest::Client::new();
    let token = match fetch_token(&client, &base_url).await {
        Ok(token) => token,
        Err(error) => {
            send_status(&events, &session_id, format!("token failed: {error:#}"));
            return;
        }
    };
    let url = terminal_ws_url(&base_url, &session_id, &token);
    let (socket, _) = match connect_async(&url).await {
        Ok(socket) => socket,
        Err(error) => {
            send_status(&events, &session_id, format!("connect failed: {error:#}"));
            return;
        }
    };
    send_status(&events, &session_id, "connected");
    let (mut write, mut read) = socket.split();
    let resize = resize_message(cols, rows);
    let _ = write.send(Message::Text(resize.into())).await;

    loop {
        tokio::select! {
            input = input_rx.recv() => {
                let Some(input) = input else {
                    break;
                };
                let message = match input {
                    TerminalInput::Bytes(bytes) => Message::Binary(bytes.into()),
                    TerminalInput::Resize { cols, rows } => Message::Text(resize_message(cols, rows).into()),
                };
                if let Err(error) = write.send(message).await {
                    send_status(&events, &session_id, format!("send failed: {error:#}"));
                    break;
                }
            }
            message = read.next() => {
                match message {
                    Some(Ok(Message::Text(text))) => {
                        let _ = events.send(TerminalEvent::Output {
                            session_id: session_id.clone(),
                            bytes: text.to_string().into_bytes(),
                        });
                    }
                    Some(Ok(Message::Binary(bytes))) => {
                        let _ = events.send(TerminalEvent::Output {
                            session_id: session_id.clone(),
                            bytes: bytes.to_vec(),
                        });
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        send_status(&events, &session_id, "disconnected");
                        break;
                    }
                    Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_))) | Some(Ok(Message::Frame(_))) => {}
                    Some(Err(error)) => {
                        send_status(&events, &session_id, format!("read failed: {error:#}"));
                        break;
                    }
                }
            }
        }
    }
}

async fn fetch_token(client: &reqwest::Client, base_url: &str) -> anyhow::Result<String> {
    #[derive(serde::Deserialize)]
    struct TokenResponse {
        token: String,
    }

    let token = client
        .get(format!("{}/token", base_url.trim_end_matches('/')))
        .send()
        .await?
        .error_for_status()?
        .json::<TokenResponse>()
        .await?;
    Ok(token.token)
}

fn terminal_ws_url(base_url: &str, session_id: &str, token: &str) -> String {
    let base = base_url.trim_end_matches('/');
    let ws_base = if let Some(rest) = base.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = base.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        base.to_string()
    };
    format!(
        "{ws_base}/terminal/{}?token={}",
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

fn resize_message(cols: u16, rows: u16) -> String {
    format!(r#"{{"type":"resize","cols":{cols},"rows":{rows}}}"#)
}

fn send_status(events: &mpsc::Sender<TerminalEvent>, session_id: &str, status: impl Into<String>) {
    let _ = events.send(TerminalEvent::Status {
        session_id: session_id.to_string(),
        status: status.into(),
    });
}

#[derive(Clone, Debug, Eq, PartialEq)]
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
            TerminalEvent::Output { session_id, bytes } => {
                self.buffer_mut(&session_id).append(&bytes);
            }
            TerminalEvent::Status { session_id, status } => {
                self.buffer_mut(&session_id).status = Some(status);
            }
        }
    }

    pub fn lines_for(&self, session_id: &str, height: usize) -> Vec<String> {
        self.buffers
            .get(session_id)
            .map(|buffer| buffer.visible_lines(height))
            .unwrap_or_default()
    }

    pub fn status_for(&self, session_id: &str) -> Option<&str> {
        self.buffers
            .get(session_id)
            .and_then(|buffer| buffer.status.as_deref())
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

#[derive(Clone, Debug, Eq, PartialEq)]
struct TerminalBuffer {
    lines: Vec<String>,
    parser_state: ParserState,
    status: Option<String>,
}

impl TerminalBuffer {
    fn append(&mut self, bytes: &[u8]) {
        let text = String::from_utf8_lossy(bytes);
        for ch in text.chars() {
            self.process_char(ch);
        }
        self.truncate();
    }

    fn process_char(&mut self, ch: char) {
        match self.parser_state {
            ParserState::Ground => self.process_ground(ch),
            ParserState::Escape => {
                self.parser_state = if ch == '[' {
                    ParserState::Csi(String::new())
                } else {
                    ParserState::Ground
                };
            }
            ParserState::Csi(ref mut params) => {
                if ('@'..='~').contains(&ch) {
                    let command = std::mem::take(params);
                    self.parser_state = ParserState::Ground;
                    self.apply_csi(&command, ch);
                } else {
                    params.push(ch);
                }
            }
        }
    }

    fn process_ground(&mut self, ch: char) {
        match ch {
            '\u{1b}' => self.parser_state = ParserState::Escape,
            '\r' => {}
            '\n' => self.lines.push(String::new()),
            '\u{8}' | '\u{7f}' => {
                let _ = self.current_line().pop();
            }
            '\t' => self.current_line().push_str("    "),
            ch if !ch.is_control() => self.current_line().push(ch),
            _ => {}
        }
    }

    fn apply_csi(&mut self, params: &str, command: char) {
        match command {
            'J' if params.ends_with('2') || params.is_empty() => {
                self.lines.clear();
                self.lines.push(String::new());
            }
            'K' => self.current_line().clear(),
            _ => {}
        }
    }

    fn current_line(&mut self) -> &mut String {
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
        self.lines.last_mut().expect("line exists")
    }

    fn visible_lines(&self, height: usize) -> Vec<String> {
        let start = self.lines.len().saturating_sub(height);
        self.lines[start..].to_vec()
    }

    fn truncate(&mut self) {
        let overflow = self.lines.len().saturating_sub(MAX_SCROLLBACK_LINES);
        if overflow > 0 {
            self.lines.drain(..overflow);
        }
    }
}

impl Default for TerminalBuffer {
    fn default() -> Self {
        Self {
            lines: vec![String::new()],
            parser_state: ParserState::Ground,
            status: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ParserState {
    Ground,
    Escape,
    Csi(String),
}

pub fn key_to_terminal_bytes(key: KeyEvent) -> Option<Vec<u8>> {
    if key.modifiers.intersects(KeyModifiers::SUPER) {
        return None;
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
        KeyCode::Left => bytes.extend_from_slice(b"\x1b[D"),
        KeyCode::Right => bytes.extend_from_slice(b"\x1b[C"),
        KeyCode::Up => bytes.extend_from_slice(b"\x1b[A"),
        KeyCode::Down => bytes.extend_from_slice(b"\x1b[B"),
        KeyCode::Home => bytes.extend_from_slice(b"\x1b[H"),
        KeyCode::End => bytes.extend_from_slice(b"\x1b[F"),
        KeyCode::PageUp => bytes.extend_from_slice(b"\x1b[5~"),
        KeyCode::PageDown => bytes.extend_from_slice(b"\x1b[6~"),
        KeyCode::Tab => bytes.push(b'\t'),
        KeyCode::BackTab => bytes.extend_from_slice(b"\x1b[Z"),
        KeyCode::Delete => bytes.extend_from_slice(b"\x1b[3~"),
        KeyCode::Insert => bytes.extend_from_slice(b"\x1b[2~"),
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
