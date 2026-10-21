//! Extra VT events belong to the session, not to individual renderers.
use capsem_proto::terminal::{TerminalEvent, TerminalEventKind, TerminalNotification};
use capsem_proto::{TERMINAL_EVENTS_PER_UPDATE, TERMINAL_METADATA_BYTES, TERMINAL_RENDER_CHUNK_BYTES};

#[derive(Default)]
pub(super) struct Callbacks {
    pub title: String,
    pub icon_name: String,
    pub bell_count: u64,
    pub last_notification: Option<TerminalNotification>,
    pub events: Vec<TerminalEvent>,
    pub replies: Vec<u8>,
}

fn text(bytes: &[u8]) -> String {
    let mut result = String::new();
    for c in String::from_utf8_lossy(bytes).chars().filter(|c| !c.is_control()) {
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
        self.bell_count += 1;
        self.event(TerminalEvent {
            kind: TerminalEventKind::Bell,
            visual,
            notification: None,
        });
    }
    fn notify(&mut self, title: &[u8], body: &[u8]) {
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
    fn reply(&mut self, reply: &[u8]) {
        if self.replies.len() + reply.len() <= TERMINAL_RENDER_CHUNK_BYTES {
            self.replies.extend_from_slice(reply);
        }
    }
}

impl vt100::Callbacks for Callbacks {
    fn audible_bell(&mut self, _: &mut vt100::Screen) {
        self.bell(false);
    }
    fn visual_bell(&mut self, _: &mut vt100::Screen) {
        self.bell(true);
    }
    fn set_window_title(&mut self, _: &mut vt100::Screen, title: &[u8]) {
        self.title = text(title);
    }
    fn set_window_icon_name(&mut self, _: &mut vt100::Screen, name: &[u8]) {
        self.icon_name = text(name);
    }
    fn unhandled_osc(&mut self, _: &mut vt100::Screen, params: &[&[u8]]) {
        match params {
            [b"9", body] => self.notify(b"", body),
            [b"777", b"notify", title, body] => self.notify(title, body),
            _ => {} // Clipboard and unknown OSC commands acquire no host authority.
        }
    }
    fn unhandled_csi(
        &mut self,
        screen: &mut vt100::Screen,
        first: Option<u8>,
        _: Option<u8>,
        params: &[&[u16]],
        c: char,
    ) {
        match (first, params, c) {
            (None, [[5]], 'n') => self.reply(b"\x1b[0n"),
            (None, [[6]], 'n') => {
                let (row, col) = screen.cursor_position();
                let (rows, cols) = screen.size();
                self.reply(format!("\x1b[{};{}R", row.min(rows - 1) + 1, col.min(cols - 1) + 1).as_bytes());
            }
            (None, [], 'c') | (None, [[0]], 'c') => self.reply(b"\x1b[?1;2c"),
            (None, [[18]], 't') => {
                let (rows, cols) = screen.size();
                self.reply(format!("\x1b[8;{rows};{cols}t").as_bytes());
            }
            _ => {}
        }
    }
}
