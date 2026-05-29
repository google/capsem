use std::time::Duration;

use anyhow::Result;

use crate::model::{
    AppState, Attention, ServiceState, ServiceStatus, SessionLifecycle, SessionStats,
    SessionSummary,
};
use crate::provider::StateProvider;

#[derive(Default)]
pub struct FixtureProvider;

impl StateProvider for FixtureProvider {
    fn load(&self) -> Result<AppState> {
        Ok(fixture_state())
    }
}

pub fn fixture_state() -> AppState {
    AppState {
        service: ServiceState {
            status: ServiceStatus::Online,
            latency: Duration::from_millis(18),
            last_event_age: Duration::from_millis(240),
            reconnect_attempt: None,
        },
        active_session_id: "profile-v2".to_string(),
        sessions: vec![
            SessionSummary {
                id: "profile-v2".to_string(),
                title: "Profile V2".to_string(),
                repo_path: Some("github.com/google/capsem".to_string()),
                profile: "corp-default".to_string(),
                branch: Some("codex/tui-control".to_string()),
                lifecycle: SessionLifecycle::Working,
                attention: Vec::new(),
                stats: SessionStats {
                    duration: Duration::from_secs(47 * 60),
                    jobs: 2,
                    events: 148,
                    tokens: 38_420,
                    cost_micros: 214_000,
                },
            },
            SessionSummary {
                id: "linux-os".to_string(),
                title: "Linux OS".to_string(),
                repo_path: Some("github.com/google/capsem-linux".to_string()),
                profile: "linux-builder".to_string(),
                branch: Some("resume-fix".to_string()),
                lifecycle: SessionLifecycle::WaitingForInput,
                attention: vec![Attention::Bell],
                stats: SessionStats {
                    duration: Duration::from_secs(2 * 60 * 60 + 11 * 60),
                    jobs: 1,
                    events: 62,
                    tokens: 12_900,
                    cost_micros: 76_000,
                },
            },
            SessionSummary {
                id: "security".to_string(),
                title: "Security".to_string(),
                repo_path: None,
                profile: "high-risk".to_string(),
                branch: None,
                lifecycle: SessionLifecycle::Suspended,
                attention: vec![Attention::ApprovalRequired, Attention::StaleData],
                stats: SessionStats {
                    duration: Duration::from_secs(19 * 60),
                    jobs: 0,
                    events: 311,
                    tokens: 91_250,
                    cost_micros: 488_000,
                },
            },
        ],
    }
}
