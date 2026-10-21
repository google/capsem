//! Session-owned terminal state. Render bytes project this state, never raw
//! workload history or its side effects.
use serde::{Deserialize, Serialize};

mod validation;
pub use validation::validate_size;
#[cfg(feature = "schema")]
mod schema;

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
    #[cfg_attr(feature = "schema", schema(schema_with = schema::text))]
    pub mouse_protocol: String,
    #[cfg_attr(feature = "schema", schema(schema_with = schema::text))]
    pub mouse_encoding: String,
}

#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalNotification {
    #[cfg_attr(feature = "schema", schema(schema_with = schema::text))]
    pub id: String,
    #[cfg_attr(feature = "schema", schema(schema_with = schema::text))]
    pub title: String,
    #[cfg_attr(feature = "schema", schema(schema_with = schema::text))]
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
    #[cfg_attr(feature = "schema", schema(schema_with = schema::cols))]
    pub cols: u16,
    #[cfg_attr(feature = "schema", schema(schema_with = schema::rows))]
    pub rows: u16,
    pub cursor: TerminalCursor,
    pub modes: TerminalModes,
    #[cfg_attr(feature = "schema", schema(schema_with = schema::text))]
    pub title: String,
    #[cfg_attr(feature = "schema", schema(schema_with = schema::text))]
    pub icon_name: String,
    pub bell_count: u64,
    pub last_notification: Option<TerminalNotification>,
    #[cfg_attr(feature = "schema", schema(schema_with = schema::events))]
    pub events: Vec<TerminalEvent>,
}

#[cfg_attr(feature = "schema", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalSummary {
    pub revision: u64,
    #[cfg_attr(feature = "schema", schema(schema_with = schema::text))]
    pub title: String,
    pub bell_count: u64,
    pub last_notification: Option<TerminalNotification>,
}

#[cfg(test)]
mod tests;
