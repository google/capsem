//! Full terminal model checkpoint, independent of event proxies and display caches.
//!
//! Callers bound serialized bytes before decoding. Restore validates the model
//! before exposing it and requires the caller's current trusted configuration.
use std::ops::Range;

use serde::{Deserialize, Serialize};

use super::cell::{checkpoint_hyperlink_counter, restore_hyperlink_counter, Cell, Flags};
use super::color::{self, Colors};
use super::{Config, TabStops, Term, TermDamageState, TermMode, KEYBOARD_MODE_STACK_MAX_DEPTH, TITLE_STACK_MAX_DEPTH};
use crate::grid::checkpoint::GridCheckpoint;
use crate::grid::Dimensions;
use crate::index::{Line, Point};
use crate::selection::Selection;
use crate::vi_mode::ViModeCursor;
use crate::vte::ansi::{CharsetIndex, CursorStyle, KeyboardModes, Rgb};

/// Caller-owned allocation and text budgets, applied to both screens together.
pub struct CheckpointLimits {
    pub max_columns: usize,
    pub max_lines: usize,
    pub max_history: usize,
    pub max_cells: usize,
    pub max_text_bytes: usize,
    pub max_combining_chars: usize,
}

impl CheckpointLimits {
    fn valid(&self) -> bool {
        self.max_columns <= i32::MAX as usize
            && self.max_lines <= i32::MAX as usize
            && self.max_history <= i32::MAX as usize - self.max_lines
    }

    fn valid_text(&self, text: &str) -> bool {
        text.len() <= self.max_text_bytes
    }

    pub(crate) fn valid_cell(&self, cell: &Cell) -> bool {
        // Alacritty retains horizontal tabs as cells for selection/copy.
        (cell.c == '\t' || !cell.c.is_control())
            && cell.flags.bits() & !Flags::all().bits() == 0
            && cell
                .zerowidth()
                .is_none_or(|chars| chars.len() <= self.max_combining_chars && chars.iter().all(|c| !c.is_control()))
            && cell
                .hyperlink()
                .is_none_or(|link| self.valid_text(link.id()) && self.valid_text(link.uri()))
    }
}

/// Versioned model state. Event proxy is rebound and damage is reset to full.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TermCheckpoint {
    version: u16,
    is_focused: bool,
    vi_mode_cursor: Point,
    selection: Option<Selection>,
    grid: GridCheckpoint,
    inactive_grid: GridCheckpoint,
    active_charset: CharsetIndex,
    tabs: Vec<bool>,
    mode: u32,
    scroll_region: Range<Line>,
    colors: Vec<Option<Rgb>>,
    cursor_style: Option<CursorStyle>,
    title: Option<String>,
    title_stack: Vec<Option<String>>,
    keyboard_mode_stack: Vec<u8>,
    inactive_keyboard_mode_stack: Vec<u8>,
    config: Config,
    hyperlink_id_suffix: u32,
}

impl<T> Term<T> {
    /// Capture both grids and all semantic fields; do not serialize host handles.
    pub fn checkpoint(&self, limits: &CheckpointLimits) -> Result<TermCheckpoint, &'static str> {
        if !limits.valid()
            || self.config.scrolling_history > limits.max_history
            || self
                .grid
                .total_lines()
                .checked_mul(self.grid.columns())
                .and_then(|cells| {
                    self.inactive_grid
                        .total_lines()
                        .checked_mul(self.inactive_grid.columns())
                        .and_then(|other| cells.checked_add(other))
                })
                .is_none_or(|cells| cells > limits.max_cells)
            || self.title.as_ref().is_some_and(|title| !limits.valid_text(title))
            || self.title_stack.len() > TITLE_STACK_MAX_DEPTH
            || self.title_stack.iter().flatten().any(|title| !limits.valid_text(title))
        {
            return Err("terminal checkpoint exceeds its budget");
        }
        let state = TermCheckpoint {
            version: 1,
            is_focused: self.is_focused,
            vi_mode_cursor: self.vi_mode_cursor.point,
            selection: self.selection.clone(),
            grid: GridCheckpoint::capture(&self.grid, limits)?,
            inactive_grid: GridCheckpoint::capture(&self.inactive_grid, limits)?,
            active_charset: self.active_charset,
            tabs: self.tabs.tabs.clone(),
            mode: self.mode.bits(),
            scroll_region: self.scroll_region.clone(),
            colors: (0..color::COUNT).map(|i| self.colors[i]).collect(),
            cursor_style: self.cursor_style,
            title: self.title.clone(),
            title_stack: self.title_stack.clone(),
            keyboard_mode_stack: self.keyboard_mode_stack.iter().map(|mode| mode.bits()).collect(),
            inactive_keyboard_mode_stack: self
                .inactive_keyboard_mode_stack
                .iter()
                .map(|mode| mode.bits())
                .collect(),
            config: self.config.clone(),
            hyperlink_id_suffix: checkpoint_hyperlink_counter(),
        };
        state.validate(&self.config, limits)?;
        Ok(state)
    }

    /// Validate before constructing native grids; never emit title, bell or host events.
    pub fn from_checkpoint(
        state: TermCheckpoint,
        config: Config,
        event_proxy: T,
        limits: &CheckpointLimits,
    ) -> Result<Self, &'static str> {
        state.validate(&config, limits)?;
        let cols = state.grid.columns();
        let lines = state.grid.lines();
        let grid = state.grid.restore();
        let inactive_grid = state.inactive_grid.restore();
        let mut colors = Colors::default();
        for (i, value) in state.colors.into_iter().enumerate() {
            colors[i] = value;
        }
        restore_hyperlink_counter(state.hyperlink_id_suffix);
        Ok(Self {
            is_focused: state.is_focused,
            vi_mode_cursor: ViModeCursor::new(state.vi_mode_cursor),
            selection: state.selection,
            grid,
            inactive_grid,
            active_charset: state.active_charset,
            tabs: TabStops { tabs: state.tabs },
            mode: TermMode::from_bits_retain(state.mode),
            scroll_region: state.scroll_region,
            colors,
            cursor_style: state.cursor_style,
            event_proxy,
            title: state.title,
            title_stack: state.title_stack,
            keyboard_mode_stack: state
                .keyboard_mode_stack
                .into_iter()
                .map(KeyboardModes::from_bits_retain)
                .collect(),
            inactive_keyboard_mode_stack: state
                .inactive_keyboard_mode_stack
                .into_iter()
                .map(KeyboardModes::from_bits_retain)
                .collect(),
            damage: TermDamageState::new(cols, lines),
            config,
        })
    }
}

impl TermCheckpoint {
    fn validate(&self, config: &Config, limits: &CheckpointLimits) -> Result<(), &'static str> {
        if !limits.valid()
            || self.version != 1
            || &self.config != config
            || config.scrolling_history > limits.max_history
            || !limits.valid_text(&config.semantic_escape_chars)
        {
            return Err("invalid terminal checkpoint version or trusted configuration");
        }
        self.grid.validate(limits)?;
        self.inactive_grid.validate(limits)?;
        let cols = self.grid.columns();
        let lines = self.grid.lines();
        let known_modes = <TermMode as bitflags::Flags>::FLAGS
            .iter()
            .filter(|flag| flag.name() != "ANY")
            .fold(0, |bits, flag| bits | flag.value().bits());
        let alt = self.mode & TermMode::ALT_SCREEN.bits() != 0;
        if self
            .grid
            .cells()
            .checked_add(self.inactive_grid.cells())
            .is_none_or(|cells| cells > limits.max_cells)
            || self.inactive_grid.columns() != cols
            || self.inactive_grid.lines() != lines
            || self.grid.history_limit() != if alt { 0 } else { config.scrolling_history }
            || self.inactive_grid.history_limit() != if alt { config.scrolling_history } else { 0 }
            || self.tabs.len() != cols
            || self.colors.len() != color::COUNT
            || self.mode & !known_modes != 0
            || self.scroll_region.start.0 < 0
            || self.scroll_region.end.0 as usize > lines
            || self.scroll_region.start >= self.scroll_region.end
            || self.title.as_ref().is_some_and(|title| !limits.valid_text(title))
            || self.title_stack.len() > TITLE_STACK_MAX_DEPTH
            || self.title_stack.iter().flatten().any(|title| !limits.valid_text(title))
            || [&self.keyboard_mode_stack, &self.inactive_keyboard_mode_stack]
                .iter()
                .any(|stack| {
                    stack.len() > KEYBOARD_MODE_STACK_MAX_DEPTH
                        || stack.iter().any(|bits| KeyboardModes::from_bits(*bits).is_none())
                })
        {
            return Err("invalid terminal checkpoint semantic state");
        }
        if self.vi_mode_cursor.line < self.grid.topmost_line()
            || self.vi_mode_cursor.line > self.grid.bottommost_line()
            || self.vi_mode_cursor.column >= cols
            || self
                .selection
                .as_ref()
                .is_some_and(|selection| !selection.checkpoint_valid(&self.grid))
        {
            return Err("invalid terminal checkpoint selection or vi cursor");
        }
        Ok(())
    }
}
