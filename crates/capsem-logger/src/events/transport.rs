//! Bounded primary routing records. Core owns policy facts and log sanitization.
use serde::Serialize;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportEventKind {
    Connect,
    ConnectResult,
    Close,
    Lifecycle,
    Probe,
    ProbeResult,
}

impl TransportEventKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Connect => "network.connect",
            Self::ConnectResult => "network.connect_result",
            Self::Close => "network.close",
            Self::Lifecycle => "network.lifecycle",
            Self::Probe => "network.probe",
            Self::ProbeResult => "network.probe_result",
        }
    }
}

#[derive(Debug, Clone)]
pub struct TransportEvent {
    pub(crate) event_id: String,
    pub(crate) timestamp_unix_ms: i64,
    pub(crate) kind: TransportEventKind,
    pub(crate) network_id: Option<String>,
    pub(crate) connection_id: Option<String>,
    pub(crate) event_json: String,
}

impl TransportEvent {
    /// Serialize already-sanitized, typed facts and validate identity before enqueue.
    pub fn new(
        event_id: String,
        timestamp_unix_ms: i64,
        kind: TransportEventKind,
        network_id: Option<Uuid>,
        connection_id: Option<Uuid>,
        facts: &impl Serialize,
    ) -> Result<Self, String> {
        if event_id.len() != 12
            || !event_id
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err("transport event requires a twelve lower-hex event ID".into());
        }
        if timestamp_unix_ms < 0
            || network_id.is_some_and(|id| id.is_nil())
            || connection_id.is_some_and(|id| id.is_nil())
        {
            return Err("invalid transport timestamp or routing identity".into());
        }
        if (kind == TransportEventKind::Lifecycle) != connection_id.is_none()
            || (kind == TransportEventKind::Lifecycle && network_id.is_none())
        {
            return Err("transport phase and routing identity disagree".into());
        }
        let event_json = serde_json::to_string(facts).map_err(|error| error.to_string())?;
        if event_json.len() > 64 * 1024 {
            return Err("transport audit facts exceed 64 KiB".into());
        }
        Ok(Self {
            event_id,
            timestamp_unix_ms,
            kind,
            network_id: network_id.map(|id| id.to_string()),
            connection_id: connection_id.map(|id| id.to_string()),
            event_json,
        })
    }

    pub fn kind(&self) -> TransportEventKind {
        self.kind
    }

    pub fn event_id(&self) -> &str {
        &self.event_id
    }
}
