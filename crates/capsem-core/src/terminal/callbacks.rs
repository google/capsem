//! Effects are bounded session data. The engine never acquires host authority.
use std::sync::{Arc, Mutex};

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use capsem_proto::terminal::{TerminalEvent, TerminalEventKind, TerminalNotification};
use capsem_proto::{
    TERMINAL_DEFAULT_COLS, TERMINAL_DEFAULT_ROWS, TERMINAL_EVENTS_PER_UPDATE, TERMINAL_METADATA_BYTES,
    TERMINAL_RENDER_CHUNK_BYTES,
};

#[derive(Clone, Default)]
pub(super) struct Proxy(pub Arc<Mutex<Callbacks>>);

pub(super) struct Callbacks {
    pub title: String,
    pub icon_name: String,
    pub bell_count: u64,
    pub last_notification: Option<TerminalNotification>,
    pub events: Vec<TerminalEvent>,
    pub replies: Vec<u8>,
    pub cols: u16,
    pub rows: u16,
}

impl Default for Callbacks {
    fn default() -> Self {
        Self {
            title: String::new(),
            icon_name: String::new(),
            bell_count: 0,
            last_notification: None,
            events: Vec::new(),
            replies: Vec::new(),
            cols: TERMINAL_DEFAULT_COLS as u16,
            rows: TERMINAL_DEFAULT_ROWS as u16,
        }
    }
}

fn text(value: &str) -> String {
    let mut result = String::new();
    for c in value.chars().filter(|c| !c.is_control()) {
        if result.len() + c.len_utf8() > TERMINAL_METADATA_BYTES {
            break;
        }
        result.push(c);
    }
    result
}

impl Callbacks {
    fn event(&mut self, event: TerminalEvent) {
        if self.events.len() < TERMINAL_EVENTS_PER_UPDATE {
            self.events.push(event);
        }
    }
    fn bell(&mut self, visual: bool) {
        self.bell_count = self.bell_count.saturating_add(1);
        self.event(TerminalEvent {
            kind: TerminalEventKind::Bell,
            visual,
            notification: None,
        });
    }
    fn notify(&mut self, title: &str, body: &str) {
        let notification = TerminalNotification {
            id: capsem_crypto::uuid_v4().to_string(),
            title: text(title),
            body: text(body),
        };
        self.last_notification = Some(notification.clone());
        self.event(TerminalEvent {
            kind: TerminalEventKind::Notification,
            visual: false,
            notification: Some(notification),
        });
    }
    fn reply(&mut self, reply: &str) {
        if self.replies.len() + reply.len() <= TERMINAL_RENDER_CHUNK_BYTES {
            self.replies.extend_from_slice(reply.as_bytes());
        }
    }
}

impl EventListener for Proxy {
    fn send_event(&self, event: Event) {
        let mut state = self.0.lock().unwrap();
        match event {
            Event::Title(title) => state.title = text(&title),
            Event::ResetTitle => state.title.clear(),
            Event::IconName(name) => state.icon_name = text(name.as_deref().unwrap_or_default()),
            Event::Bell => state.bell(false),
            Event::VisualBell => state.bell(true),
            Event::Notification { title, body } => state.notify(&title, &body),
            Event::PtyWrite(reply) => state.reply(&reply),
            Event::ColorRequest(index, format) => state.reply(&format(super::projection::default_color(index))),
            Event::TextAreaSizeRequest(format) => {
                let size = WindowSize {
                    num_lines: state.rows,
                    num_cols: state.cols,
                    cell_width: 0,
                    cell_height: 0,
                };
                state.reply(&format(size));
            }
            // Guest requests never read/write the host clipboard, spawn a
            // process, open a URL or shut down the session owner.
            Event::ClipboardStore(..)
            | Event::ClipboardLoad(..)
            | Event::Exit
            | Event::ChildExit(..)
            | Event::MouseCursorDirty
            | Event::CursorBlinkingChange
            | Event::Wakeup => {}
        }
    }
}
