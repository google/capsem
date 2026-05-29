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
            control_message: None,
        },
        active_session_id: "profile-v2".to_string(),
        sessions: vec![
            SessionSummary {
                id: "profile-v2".to_string(),
                title: "Profile V2".to_string(),
                repo_path: Some("github.com/google/capsem".to_string()),
                profile: "corp-default".to_string(),
                branch: Some("codex/tui-control".to_string()),
                persistent: true,
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
                persistent: true,
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
        ],
    }
}
