//! Logical rows and both cursors; upstream Grid serde intentionally skips cursors.
use serde::{Deserialize, Serialize};

use super::row::Row;
use super::storage::Storage;
use super::{Cursor, Dimensions, Grid};
use crate::index::{Column, Line};
use crate::term::cell::{Cell, Flags};
use crate::term::checkpoint::CheckpointLimits;
use crate::term::{MIN_COLUMNS, MIN_SCREEN_LINES};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GridCheckpoint {
    rows: Vec<Row<Cell>>,
    columns: usize,
    lines: usize,
    display_offset: usize,
    max_scroll_limit: usize,
    cursor: Cursor<Cell>,
    saved_cursor: Cursor<Cell>,
}

impl Dimensions for GridCheckpoint {
    fn total_lines(&self) -> usize {
        self.rows.len()
    }
    fn screen_lines(&self) -> usize {
        self.lines
    }
    fn columns(&self) -> usize {
        self.columns
    }
}

impl GridCheckpoint {
    pub fn capture(grid: &Grid<Cell>, limits: &CheckpointLimits) -> Result<Self, &'static str> {
        let count = grid.raw.len();
        if count
            .checked_mul(grid.columns)
            .is_none_or(|cells| cells > limits.max_cells)
        {
            return Err("grid checkpoint exceeds cell budget");
        }
        // Discard only invisible allocation-cache rows. Canonical logical rows
        // are serialized bottom-first with ring offset zero, retaining history.
        let rows = (-(count as i32 - grid.lines as i32)..grid.lines as i32)
            .rev()
            .map(|line| grid.raw[Line(line)].clone())
            .collect();
        let checkpoint = Self {
            rows,
            columns: grid.columns,
            lines: grid.lines,
            display_offset: grid.display_offset,
            max_scroll_limit: grid.max_scroll_limit,
            cursor: grid.cursor.clone(),
            saved_cursor: grid.saved_cursor.clone(),
        };
        checkpoint.validate(limits)?;
        Ok(checkpoint)
    }

    pub fn cells(&self) -> usize {
        self.rows.len() * self.columns
    }
    pub fn columns(&self) -> usize {
        self.columns
    }
    pub fn lines(&self) -> usize {
        self.lines
    }
    pub fn history_limit(&self) -> usize {
        self.max_scroll_limit
    }

    pub fn validate(&self, limits: &CheckpointLimits) -> Result<(), &'static str> {
        if self.columns < MIN_COLUMNS
            || self.columns > limits.max_columns
            || self.lines < MIN_SCREEN_LINES
            || self.lines > limits.max_lines
            || self.rows.len() < self.lines
            || self.rows.len() - self.lines > self.max_scroll_limit
            || self.max_scroll_limit > limits.max_history
            || self.display_offset > self.rows.len() - self.lines
            || self
                .rows
                .len()
                .checked_mul(self.columns)
                .is_none_or(|cells| cells > limits.max_cells)
        {
            return Err("invalid checkpoint grid dimensions or history");
        }
        for cursor in [&self.cursor, &self.saved_cursor] {
            if cursor.point.line.0 < 0
                || cursor.point.line.0 as usize >= self.lines
                || cursor.point.column.0 >= self.columns
                || !limits.valid_cell(&cursor.template)
            {
                return Err("invalid checkpoint cursor");
            }
        }
        for row in &self.rows {
            if row.len() != self.columns || row.occ > self.columns {
                return Err("invalid checkpoint row");
            }
            for column in 0..self.columns {
                let cell = &row[Column(column)];
                if !limits.valid_cell(cell) {
                    return Err("invalid checkpoint cell");
                }
                if cell.flags.contains(Flags::WIDE_CHAR)
                    && (column + 1 == self.columns || !row[Column(column + 1)].flags.contains(Flags::WIDE_CHAR_SPACER))
                {
                    return Err("invalid checkpoint wide character");
                }
                if cell.flags.contains(Flags::WIDE_CHAR_SPACER)
                    && (column == 0 || !row[Column(column - 1)].flags.contains(Flags::WIDE_CHAR))
                {
                    return Err("invalid checkpoint wide character spacer");
                }
            }
        }
        Ok(())
    }

    pub fn restore(self) -> Grid<Cell> {
        Grid {
            raw: Storage::from_checkpoint(self.rows, self.lines),
            columns: self.columns,
            lines: self.lines,
            display_offset: self.display_offset,
            max_scroll_limit: self.max_scroll_limit,
            cursor: self.cursor,
            saved_cursor: self.saved_cursor,
        }
    }
}
