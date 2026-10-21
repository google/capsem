//! Validate owner-provided state before it can allocate a client screen.
use super::*;
use crate::{
    TERMINAL_EVENTS_PER_UPDATE, TERMINAL_MAX_CELLS, TERMINAL_MAX_COLS, TERMINAL_MAX_ROWS, TERMINAL_METADATA_BYTES,
};

pub fn validate_size(cols: u16, rows: u16) -> Result<(), &'static str> {
    if cols == 0
        || rows == 0
        || usize::from(cols) > TERMINAL_MAX_COLS
        || usize::from(rows) > TERMINAL_MAX_ROWS
        || usize::from(cols) * usize::from(rows) > TERMINAL_MAX_CELLS
    {
        return Err("terminal dimensions exceed the supported grid");
    }
    Ok(())
}
fn text(value: &str) -> Result<(), &'static str> {
    if value.len() > TERMINAL_METADATA_BYTES || value.chars().any(char::is_control) {
        return Err("terminal metadata exceeds the supported text budget");
    }
    Ok(())
}
impl TerminalNotification {
    fn validate(&self) -> Result<(), &'static str> {
        text(&self.id)?;
        text(&self.title)?;
        text(&self.body)
    }
}
impl TerminalSummary {
    pub fn validate(&self) -> Result<(), &'static str> {
        text(&self.title)?;
        if let Some(notification) = &self.last_notification {
            notification.validate()?;
        }
        Ok(())
    }
}
impl TerminalState {
    pub fn validate(&self) -> Result<(), &'static str> {
        validate_size(self.cols, self.rows)?;
        if self.cursor.row >= self.rows || self.cursor.col > self.cols {
            return Err("terminal cursor is outside the supported grid");
        }
        text(&self.title)?;
        text(&self.icon_name)?;
        text(&self.modes.mouse_protocol)?;
        text(&self.modes.mouse_encoding)?;
        if let Some(notification) = &self.last_notification {
            notification.validate()?;
        }
        if self.events.len() > TERMINAL_EVENTS_PER_UPDATE {
            return Err("terminal event budget exceeded");
        }
        for event in &self.events {
            match (&event.kind, &event.notification) {
                (TerminalEventKind::Bell, None) => {}
                (TerminalEventKind::Notification, Some(notification)) => notification.validate()?,
                _ => return Err("terminal event payload does not match its kind"),
            }
        }
        Ok(())
    }
}
