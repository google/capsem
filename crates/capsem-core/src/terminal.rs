//! One Rust session owns emulation, dimensions and effects. Thin viewers receive
//! coherent display projections, never a suffix of workload control history.
use std::sync::{Arc, Mutex};

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::{Config, LineDamageBounds, Osc52, TermDamage, TermMode};
use alacritty_terminal::vte::ansi::Processor;
use alacritty_terminal::Term;
use capsem_proto::terminal::{validate_size, TerminalCursor, TerminalModes, TerminalState, TerminalSummary};
use capsem_proto::{TERMINAL_COMBINING_CHARS, TERMINAL_DEFAULT_COLS, TERMINAL_DEFAULT_ROWS, TERMINAL_HISTORY_CELLS};
use tokio::sync::{broadcast, watch};

mod callbacks;
mod checkpoint;
pub use checkpoint::{CheckpointError, MAX_CHECKPOINT_BYTES};
mod osc_budget;
mod projection;
use callbacks::Proxy;

#[derive(Clone, Debug)]
pub struct TerminalUpdate {
    pub state: TerminalState,
    pub render: Vec<u8>,
}

pub struct SessionTerminal {
    inner: Mutex<Inner>,
}

struct Inner {
    term: Term<Proxy>,
    parser: Processor,
    effects: Proxy,
    osc_budget: osc_budget::OscBudget,
    revision: u64,
    next_view: u64,
    controller: Option<u64>,
    accepting_views: bool,
    updates: broadcast::Sender<Arc<TerminalUpdate>>,
    summary: watch::Sender<TerminalSummary>,
}

fn options(cols: usize) -> Config {
    Config {
        scrolling_history: TERMINAL_HISTORY_CELLS / cols,
        max_combining_chars: Some(TERMINAL_COMBINING_CHARS),
        osc52: Osc52::Disabled,
        ..Config::default()
    }
}

fn size(cols: u16, rows: u16) -> Result<(), &'static str> {
    validate_size(cols, rows)?;
    if usize::from(cols) < alacritty_terminal::term::MIN_COLUMNS {
        return Err("terminal needs at least two columns");
    }
    Ok(())
}

struct GridSize(usize, usize);
impl Dimensions for GridSize {
    fn total_lines(&self) -> usize {
        self.1
    }
    fn screen_lines(&self) -> usize {
        self.1
    }
    fn columns(&self) -> usize {
        self.0
    }
}

impl SessionTerminal {
    pub fn new(capacity: usize) -> Arc<Self> {
        let (updates, _) = broadcast::channel(capacity);
        let (summary, _) = watch::channel(TerminalSummary::default());
        let effects = Proxy::default();
        let dimensions = GridSize(TERMINAL_DEFAULT_COLS, TERMINAL_DEFAULT_ROWS);
        Arc::new(Self {
            inner: Mutex::new(Inner {
                term: Term::new(options(TERMINAL_DEFAULT_COLS), &dimensions, effects.clone()),
                parser: Processor::new(),
                effects,
                osc_budget: Default::default(),
                revision: 0,
                next_view: 0,
                controller: None,
                accepting_views: true,
                updates,
                summary,
            }),
        })
    }

    /// Interpret once without viewers. Only owner-generated replies return to
    /// the PTY; queries and other effects never reach a renderer for replay.
    pub fn publish(&self, data: impl AsRef<[u8]>) -> Vec<u8> {
        let mut inner = self.inner.lock().unwrap();
        let alternate = inner.term.mode().contains(TermMode::ALT_SCREEN);
        let Inner {
            term,
            parser,
            osc_budget,
            ..
        } = &mut *inner;
        osc_budget.process(data.as_ref(), |bytes| parser.advance(term, bytes));
        inner.finish_update(alternate)
    }

    /// The transport drives this deadline even when the PTY becomes quiet.
    pub fn sync_deadline(&self) -> Option<std::time::Instant> {
        self.inner.lock().unwrap().parser.sync_timeout().sync_timeout()
    }

    pub fn expire_sync(&self) -> Vec<u8> {
        self.expire_sync_at(std::time::Instant::now())
    }

    fn expire_sync_at(&self, now: std::time::Instant) -> Vec<u8> {
        let mut inner = self.inner.lock().unwrap();
        if !inner
            .parser
            .sync_timeout()
            .sync_timeout()
            .is_some_and(|deadline| deadline <= now)
        {
            return Vec::new();
        }
        let alternate = inner.term.mode().contains(TermMode::ALT_SCREEN);
        let Inner { parser, term, .. } = &mut *inner;
        parser.stop_sync(term);
        inner.finish_update(alternate)
    }

    /// Snapshot and subscription share publication's lock: no missing bytes or
    /// duplicated effects at attachment, regardless of the previous viewer.
    pub fn subscribe(&self) -> (u64, TerminalUpdate, broadcast::Receiver<Arc<TerminalUpdate>>) {
        let mut inner = self.inner.lock().unwrap();
        inner.next_view += 1;
        (
            inner.next_view,
            inner.update(true, &inner.full_damage()),
            inner.updates.subscribe(),
        )
    }

    pub fn focus(&self, view: u64, cols: u16, rows: u16) -> Result<bool, &'static str> {
        size(cols, rows)?;
        let mut inner = self.inner.lock().unwrap();
        if !inner.accepting_views {
            return Err("terminal is suspending");
        }
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
        size(cols, rows)?;
        let mut inner = self.inner.lock().unwrap();
        if !inner.accepting_views {
            return Err("terminal is suspending");
        }
        if inner.controller != Some(view) {
            return Ok(false);
        }
        Ok(inner.resize(cols, rows))
    }

    pub fn subscribe_summary(&self) -> watch::Receiver<TerminalSummary> {
        self.inner.lock().unwrap().summary.subscribe()
    }
    pub fn controls(&self, view: u64) -> bool {
        let inner = self.inner.lock().unwrap();
        inner.accepting_views && inner.controller == Some(view)
    }
    /// Fence viewer mutation while the owner settles accepted transport
    /// bytes. A restored continuation creates fresh authority; a failed
    /// suspend may reopen the existing views after rolling back the VM.
    pub fn seal_views(&self) {
        self.inner.lock().unwrap().accepting_views = false;
    }
    /// Reopen viewer mutation after the owner has rolled back suspend.
    /// A controller released during the fence is not reacquired.
    pub fn resume_views(&self) {
        self.inner.lock().unwrap().accepting_views = true;
    }
    pub fn release(&self, view: u64) {
        let mut inner = self.inner.lock().unwrap();
        if inner.controller == Some(view) {
            inner.controller = None;
        }
    }
}

impl Inner {
    fn finish_update(&mut self, alternate: bool) -> Vec<u8> {
        self.revision += 1;
        let reset = alternate != self.term.mode().contains(TermMode::ALT_SCREEN);
        let damage = match self.term.damage() {
            TermDamage::Full => self.full_damage(),
            TermDamage::Partial(lines) => lines.collect(),
        };
        let (events, replies) = {
            let mut effects = self.effects.0.lock().unwrap();
            (
                std::mem::take(&mut effects.events),
                std::mem::take(&mut effects.replies),
            )
        };
        let mut update = self.update(reset, &damage);
        update.state.events = events;
        self.term.reset_damage();
        self.publish_summary();
        let _ = self.updates.send(Arc::new(update)); // channel-closed-ok: session-terminal-retains-state
        replies
    }

    fn full_damage(&self) -> Vec<LineDamageBounds> {
        (0..self.term.screen_lines())
            .map(|line| LineDamageBounds::new(line, 0, self.term.columns() - 1))
            .collect()
    }

    fn publish_summary(&self) {
        let effects = self.effects.0.lock().unwrap();
        let summary = TerminalSummary {
            revision: self.revision,
            title: effects.title.clone(),
            bell_count: effects.bell_count,
            last_notification: effects.last_notification.clone(),
        };
        drop(effects);
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
        if (self.term.columns(), self.term.screen_lines()) == (usize::from(cols), usize::from(rows)) {
            return false;
        }
        self.term.set_options(options(usize::from(cols)));
        self.term.resize(GridSize(usize::from(cols), usize::from(rows)));
        {
            let mut effects = self.effects.0.lock().unwrap();
            effects.cols = cols;
            effects.rows = rows;
        }
        self.revision += 1;
        let _ = self.updates.send(Arc::new(self.update(true, &self.full_damage()))); // channel-closed-ok: session-terminal-retains-state
        self.term.reset_damage();
        true
    }

    fn update(&self, reset: bool, damage: &[LineDamageBounds]) -> TerminalUpdate {
        let effects = self.effects.0.lock().unwrap();
        let mode = self.term.mode();
        let cursor = &self.term.grid().cursor;
        let mouse = if mode.contains(TermMode::MOUSE_MOTION) {
            "AnyMotion"
        } else if mode.contains(TermMode::MOUSE_DRAG) {
            "ButtonMotion"
        } else if mode.contains(TermMode::MOUSE_REPORT_CLICK) {
            "PressRelease"
        } else {
            "None"
        };
        let encoding = if mode.contains(TermMode::SGR_MOUSE) {
            "Sgr"
        } else if mode.contains(TermMode::UTF8_MOUSE) {
            "Utf8"
        } else {
            "Default"
        };
        let state = TerminalState {
            revision: self.revision,
            reset,
            cols: self.term.columns() as u16,
            rows: self.term.screen_lines() as u16,
            cursor: TerminalCursor {
                row: cursor.point.line.0 as u16,
                col: (cursor.point.column.0 + usize::from(cursor.input_needs_wrap)) as u16,
                hidden: !mode.contains(TermMode::SHOW_CURSOR),
            },
            modes: TerminalModes {
                alternate_screen: mode.contains(TermMode::ALT_SCREEN),
                application_cursor: mode.contains(TermMode::APP_CURSOR),
                application_keypad: mode.contains(TermMode::APP_KEYPAD),
                bracketed_paste: mode.contains(TermMode::BRACKETED_PASTE),
                mouse_protocol: mouse.into(),
                mouse_encoding: encoding.into(),
            },
            title: effects.title.clone(),
            icon_name: effects.icon_name.clone(),
            bell_count: effects.bell_count,
            last_notification: effects.last_notification.clone(),
            events: Vec::new(),
        };
        drop(effects);
        TerminalUpdate {
            state,
            render: projection::render(&self.term, reset, damage),
        }
    }
}

#[cfg(test)]
mod tests;
