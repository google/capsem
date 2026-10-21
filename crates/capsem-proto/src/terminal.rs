//! Session-owned terminal state. Render bytes project this state, never raw
//! workload history or its side effects.
use serde::{Deserialize, Serialize};

#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalCursor {
    pub row: u16,
    pub col: u16,
    pub hidden: bool,
}

#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalModes {
    pub alternate_screen: bool,
    pub application_cursor: bool,
    pub application_keypad: bool,
    pub bracketed_paste: bool,
    pub mouse_protocol: String,
    pub mouse_encoding: String,
}

#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalNotification {
    pub id: String,
    pub title: String,
    pub body: String,
}

#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalEventKind {
    Bell,
    Notification,
}

#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalEvent {
    pub kind: TerminalEventKind,
    pub visual: bool,
    pub notification: Option<TerminalNotification>,
}

/// Metadata precedes the corresponding display projection. A reset replaces
/// the display at these dimensions. Snapshot events are always empty.
#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalState {
    pub revision: u64,
    pub reset: bool,
    pub cols: u16,
    pub rows: u16,
    pub cursor: TerminalCursor,
    pub modes: TerminalModes,
    pub title: String,
    pub icon_name: String,
    pub bell_count: u64,
    pub last_notification: Option<TerminalNotification>,
    pub events: Vec<TerminalEvent>,
}

#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalSummary {
    pub revision: u64,
    pub title: String,
    pub bell_count: u64,
    pub last_notification: Option<TerminalNotification>,
}

#[cfg(test)]
mod tests;
