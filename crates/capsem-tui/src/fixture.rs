use std::time::Duration;

use anyhow::Result;

use crate::model::{
    AppState, Attention, ProfileOption, ServiceState, ServiceStatus, SessionLifecycle,
    SessionStats, SessionSummary,
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
        profiles: vec![
            ProfileOption {
                id: "corp-default".to_string(),
                name: "Corp Default".to_string(),
                description: Some("default profile".to_string()),
                is_default: true,
            },
            ProfileOption {
                id: "linux-builder".to_string(),
                name: "Linux Builder".to_string(),
                description: Some("kernel and distro work".to_string()),
                is_default: false,
            },
        ],
        sessions: vec![
            SessionSummary {
                id: "profile-v2".to_string(),
                title: "Profile V2".to_string(),
                repo_path: Some("github.com/google/capsem".to_string()),
                profile: "corp-default".to_string(),
                profile_status: Some("current".to_string()),
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
                    configured_ram_mb: Some(4096),
                    configured_vcpus: Some(4),
                    host_process_rss_bytes: Some(384 * 1024 * 1024),
                    host_cpu_time_micros: Some(2_460_000),
                    block_queue_notifications_total: Some(5_876),
                    block_queue_drains_total: Some(1_639),
                    block_read_ops_total: Some(8_580),
                    block_write_ops_total: Some(3),
                    block_bytes_read_total: Some(31_394_816),
                    block_bytes_written_total: Some(4_096),
                },
            },
            SessionSummary {
                id: "linux-os".to_string(),
                title: "Linux OS".to_string(),
                repo_path: Some("github.com/google/capsem-linux".to_string()),
                profile: "linux-builder".to_string(),
                profile_status: Some("current".to_string()),
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
                    configured_ram_mb: Some(4096),
                    configured_vcpus: Some(4),
                    host_process_rss_bytes: None,
                    host_cpu_time_micros: None,
                    block_queue_notifications_total: None,
                    block_queue_drains_total: None,
                    block_read_ops_total: None,
                    block_write_ops_total: None,
                    block_bytes_read_total: None,
                    block_bytes_written_total: None,
                },
            },
        ],
    }
}

pub fn offline_state() -> AppState {
    AppState {
        service: ServiceState {
            status: ServiceStatus::Offline,
            latency: Duration::ZERO,
            last_event_age: Duration::ZERO,
            reconnect_attempt: Some(1),
            control_message: None,
        },
        active_session_id: String::new(),
        profiles: Vec::new(),
        sessions: Vec::new(),
    }
}
