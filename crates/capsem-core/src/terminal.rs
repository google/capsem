//! The session's terminal, independent of any attached UI. All PTY bytes are
//! interpreted here. Subscribers receive a coherent display projection, never
//! an arbitrary byte tail whose interpretation depends on their window size.
use std::sync::{Arc, Mutex};

use capsem_proto::terminal::{validate_size, TerminalCursor, TerminalModes, TerminalState, TerminalSummary};
use capsem_proto::{TERMINAL_DEFAULT_COLS, TERMINAL_DEFAULT_ROWS};
use tokio::sync::{broadcast, watch};

mod callbacks;
mod osc_budget;
use callbacks::Callbacks;

#[derive(Clone, Debug)]
pub struct TerminalUpdate {
    pub state: TerminalState,
    pub render: Vec<u8>,
}

pub struct SessionTerminal {
    inner: Mutex<Inner>,
}

struct Inner {
    parser: vt100::Parser<Callbacks>,
    osc_budget: osc_budget::OscBudget,
    revision: u64,
    next_view: u64,
    controller: Option<u64>,
    updates: broadcast::Sender<Arc<TerminalUpdate>>,
    summary: watch::Sender<TerminalSummary>,
}

impl SessionTerminal {
    pub fn new(capacity: usize) -> Arc<Self> {
        let (updates, _) = broadcast::channel(capacity);
        let (summary, _) = watch::channel(TerminalSummary::default());
        Arc::new(Self {
            inner: Mutex::new(Inner {
                parser: vt100::Parser::new_with_callbacks(
                    TERMINAL_DEFAULT_ROWS as u16,
                    TERMINAL_DEFAULT_COLS as u16,
                    0,
                    Callbacks::default(),
                ),
                osc_budget: Default::default(),
                revision: 0,
                next_view: 0,
                controller: None,
                updates,
                summary,
            }),
        })
    }

    /// Interpret once, even with no viewers. Replies to terminal queries go
    /// back to the PTY once; renderers never see those requests.
    pub fn publish(&self, data: impl AsRef<[u8]>) -> Vec<u8> {
        let mut inner = self.inner.lock().unwrap();
        let previous = inner.parser.screen().clone();
        let Inner { parser, osc_budget, .. } = &mut *inner;
        osc_budget.process(data.as_ref(), |bytes| parser.process(bytes));
        inner.revision += 1;
        let reset = previous.alternate_screen() != inner.parser.screen().alternate_screen();
        let mut update = inner.update(reset, Some(&previous));
        update.state.events = std::mem::take(&mut inner.parser.callbacks_mut().events);
        let replies = std::mem::take(&mut inner.parser.callbacks_mut().replies);
        inner.publish_summary();
        // No viewer is normal: the parsed screen remains authoritative.
        let _ = inner.updates.send(Arc::new(update)); // channel-closed-ok: session-terminal-retains-state
        replies
    }

    /// Snapshot and live subscription share publication's lock. Neither a gap
    /// nor duplicated output/events can occur at the attach boundary.
    pub fn subscribe(&self) -> (u64, TerminalUpdate, broadcast::Receiver<Arc<TerminalUpdate>>) {
        let mut inner = self.inner.lock().unwrap();
        inner.next_view += 1;
        (inner.next_view, inner.update(true, None), inner.updates.subscribe())
    }

    pub fn focus(&self, view: u64, cols: u16, rows: u16) -> Result<bool, &'static str> {
        validate_size(cols, rows)?;
        let mut inner = self.inner.lock().unwrap();
        if view == 0 || view > inner.next_view {
            return Err("unknown terminal view");
        }
        let newly_controlling = inner.controller != Some(view);
        inner.controller = Some(view);
        let resized = inner.resize(cols, rows);
        drop(inner);
        Ok(newly_controlling || resized)
    }

    pub fn resize(&self, view: u64, cols: u16, rows: u16) -> Result<bool, &'static str> {
        validate_size(cols, rows)?;
        let mut inner = self.inner.lock().unwrap();
        if inner.controller != Some(view) {
            return Ok(false);
        }
        Ok(inner.resize(cols, rows))
    }

    pub fn subscribe_summary(&self) -> watch::Receiver<TerminalSummary> {
        self.inner.lock().unwrap().summary.subscribe()
    }

    pub fn controls(&self, view: u64) -> bool {
        self.inner.lock().unwrap().controller == Some(view)
    }

    pub fn release(&self, view: u64) {
        let mut inner = self.inner.lock().unwrap();
        if inner.controller == Some(view) {
            inner.controller = None;
        }
    }
}

impl Inner {
    fn publish_summary(&self) {
        let callbacks = self.parser.callbacks();
        let summary = TerminalSummary {
            revision: self.revision,
            title: callbacks.title.clone(),
            bell_count: callbacks.bell_count,
            last_notification: callbacks.last_notification.clone(),
        };
        self.summary.send_if_modified(|current| {
            if (current.title.as_str(), current.bell_count, &current.last_notification)
                == (summary.title.as_str(), summary.bell_count, &summary.last_notification)
            {
                return false;
            }
            *current = summary;
            true
        });
    }

    fn resize(&mut self, cols: u16, rows: u16) -> bool {
        if self.parser.screen().size() == (rows, cols) {
            return false;
        }
        self.parser.screen_mut().set_size(rows, cols);
        self.revision += 1;
        let _ = self.updates.send(Arc::new(self.update(true, None))); // channel-closed-ok: session-terminal-retains-state
        true
    }

    fn update(&self, reset: bool, previous: Option<&vt100::Screen>) -> TerminalUpdate {
        let screen = self.parser.screen();
        let callbacks = self.parser.callbacks();
        let (rows, cols) = screen.size();
        let (row, col) = screen.cursor_position();
        let render = if let Some(previous) = previous.filter(|_| !reset) {
            screen.state_diff(previous)
        } else {
            let mut render = b"\x1bc".to_vec();
            if screen.alternate_screen() {
                render.extend_from_slice(b"\x1b[?1049h");
            }
            render.extend(screen.state_formatted());
            render
        };
        TerminalUpdate {
            state: TerminalState {
                revision: self.revision,
                reset,
                cols,
                rows,
                cursor: TerminalCursor {
                    row,
                    col,
                    hidden: screen.hide_cursor(),
                },
                modes: TerminalModes {
                    alternate_screen: screen.alternate_screen(),
                    application_cursor: screen.application_cursor(),
                    application_keypad: screen.application_keypad(),
                    bracketed_paste: screen.bracketed_paste(),
                    mouse_protocol: format!("{:?}", screen.mouse_protocol_mode()),
                    mouse_encoding: format!("{:?}", screen.mouse_protocol_encoding()),
                },
                title: callbacks.title.clone(),
                icon_name: callbacks.icon_name.clone(),
                bell_count: callbacks.bell_count,
                last_notification: callbacks.last_notification.clone(),
                events: Vec::new(),
            },
            render,
        }
    }
}

#[cfg(test)]
mod tests;
