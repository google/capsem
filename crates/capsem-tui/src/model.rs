use std::time::Duration;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppState {
    pub service: ServiceState,
    pub active_session_id: String,
    pub sessions: Vec<SessionSummary>,
    pub profiles: Vec<ProfileOption>,
}

impl AppState {
    pub fn active_session(&self) -> Option<&SessionSummary> {
        self.sessions
            .iter()
            .find(|session| session.id == self.active_session_id)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProfileOption {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub is_default: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServiceState {
    pub status: ServiceStatus,
    pub latency: Duration,
    pub last_event_age: Duration,
    pub reconnect_attempt: Option<u32>,
    pub control_message: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceStatus {
    Online,
    Reconnecting,
    Stale,
    Offline,
    Degraded,
    Failed,
}

impl ServiceStatus {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Online => "online",
            Self::Reconnecting => "reconnecting",
            Self::Stale => "stale",
            Self::Offline => "offline",
            Self::Degraded => "degraded",
            Self::Failed => "failed",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionSummary {
    pub id: String,
    pub title: String,
    pub repo_path: Option<String>,
    pub profile: String,
    pub profile_status: Option<String>,
    pub can_resume: bool,
    pub resume_blocked_reason: Option<String>,
    pub branch: Option<String>,
    pub persistent: bool,
    pub lifecycle: SessionLifecycle,
    pub attention: Vec<Attention>,
    pub stats: SessionStats,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionLifecycle {
    Idle,
    Suspended,
    Working,
    WaitingForInput,
    Failed,
}

impl SessionLifecycle {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Suspended => "suspended",
            Self::Working => "working",
            Self::WaitingForInput => "waiting",
            Self::Failed => "failed",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Attention {
    Bell,
    ApprovalRequired,
    PolicyDeny,
    CredentialIssue,
    StaleData,
}

impl Attention {
    pub const fn marker(self) -> &'static str {
        match self {
            Self::Bell => "bell",
            Self::ApprovalRequired => "approval",
            Self::PolicyDeny => "policy",
            Self::CredentialIssue => "creds",
            Self::StaleData => "stale",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionStats {
    pub duration: Duration,
    pub jobs: u16,
    pub events: u32,
    pub tokens: u64,
    pub cost_micros: u64,
}
