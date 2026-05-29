use anyhow::Result;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::{Frame, Terminal};

use crate::model::{AppState, ServiceStatus, SessionLifecycle, SessionSummary};

const MAX_VISIBLE_TABS: usize = 4;

pub fn render(frame: &mut Frame<'_>, state: &AppState) {
    let root = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(root);

    render_tabs(frame, state, chunks[0]);
    render_active_desktop(frame, state, chunks[1]);
    render_status_bar(frame, state, chunks[2]);
}

pub fn render_snapshot(state: &AppState, width: u16, height: u16) -> Result<String> {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend)?;
    terminal.draw(|frame| render(frame, state))?;
    Ok(buffer_to_string(terminal.backend().buffer()))
}

fn render_status_bar(frame: &mut Frame<'_>, state: &AppState, area: Rect) {
    let service = &state.service;
    let (waiting, running, idle) = session_counts(state);
    let mut left = vec![
        Span::raw(" "),
        service_dot(service.status, service.latency.as_millis()),
        Span::raw(format!(" {}ms ", service.latency.as_millis())),
        Span::raw(format!("[w/r/i {waiting}/{running}/{idle}] ")),
        Span::raw(format!("[terminals {}]", state.sessions.len())),
    ];
    if let Some(attempt) = service.reconnect_attempt {
        left.push(Span::raw(format!(" reconnect#{attempt}")));
    }
    let left_width = spans_width(&left);
    let right = state
        .active_session()
        .map(active_stats)
        .unwrap_or_else(|| "no session".to_string());
    let right_width = right.chars().count();
    let area_width = area.width as usize;
    let gap = area_width.saturating_sub(left_width + right_width).max(1);
    left.push(Span::raw(" ".repeat(gap)));
    left.push(Span::raw(right));
    frame.render_widget(Paragraph::new(Line::from(left)), area);
}

fn render_tabs(frame: &mut Frame<'_>, state: &AppState, area: Rect) {
    let active_index = state
        .sessions
        .iter()
        .position(|session| session.id == state.active_session_id)
        .unwrap_or_default();
    let visible = visible_tab_range(state.sessions.len(), active_index);
    let mut spans = Vec::new();
    if visible.start > 0 {
        spans.push(Span::styled(" < ", Style::default().fg(Color::DarkGray)));
    } else {
        spans.push(Span::raw("   "));
    }
    for (offset, session) in state.sessions[visible.clone()].iter().enumerate() {
        let index = visible.start + offset;
        let mut style = Style::default().fg(Color::Gray);
        if index == active_index {
            style = Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD);
        }
        spans.push(Span::styled(tab_label(session), style));
        spans.push(Span::raw(" "));
    }
    if visible.end < state.sessions.len() {
        spans.push(Span::styled("> ", Style::default().fg(Color::DarkGray)));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_active_desktop(frame: &mut Frame<'_>, state: &AppState, area: Rect) {
    let Some(session) = state.active_session() else {
        frame.render_widget(Paragraph::new("No active session."), area);
        return;
    };

    let repo = session.repo_path.as_deref().unwrap_or("no repo");
    let branch = session.branch.as_deref().unwrap_or("no branch");
    let text = vec![
        Line::from(format!("{}  {}  {}", session.title, repo, branch)),
        Line::from(""),
        Line::from("$ cargo test -p capsem-tui"),
        Line::from("running 2 tests"),
        Line::from("test fixture_models_global_service_state_and_session_indicators ... ok"),
        Line::from("test snapshot_contains_light_bar_tabs_and_active_desktop ... ok"),
        Line::from(""),
        Line::from(
            "Fixture terminal surface. Real attach and HTTP state arrive in later sub-sprints.",
        ),
    ];
    let paragraph = Paragraph::new(text).wrap(Wrap { trim: true });
    frame.render_widget(paragraph, area);
}

fn service_dot(status: ServiceStatus, latency_ms: u128) -> Span<'static> {
    let color = match status {
        ServiceStatus::Online if latency_ms < 100 => Color::Green,
        ServiceStatus::Online | ServiceStatus::Reconnecting | ServiceStatus::Stale => Color::Yellow,
        ServiceStatus::Degraded => Color::Yellow,
        ServiceStatus::Offline | ServiceStatus::Failed => Color::Red,
    };
    Span::styled("●", Style::default().fg(color))
}

fn session_counts(state: &AppState) -> (usize, usize, usize) {
    state
        .sessions
        .iter()
        .fold((0, 0, 0), |mut counts, session| {
            match session.lifecycle {
                SessionLifecycle::WaitingForInput => counts.0 += 1,
                SessionLifecycle::Working => counts.1 += 1,
                SessionLifecycle::Idle | SessionLifecycle::Suspended => counts.2 += 1,
                SessionLifecycle::Failed => {}
            }
            counts
        })
}

fn visible_tab_range(len: usize, active_index: usize) -> std::ops::Range<usize> {
    if len <= MAX_VISIBLE_TABS {
        return 0..len;
    }
    let half = MAX_VISIBLE_TABS / 2;
    let start = active_index
        .saturating_sub(half)
        .min(len - MAX_VISIBLE_TABS);
    start..start + MAX_VISIBLE_TABS
}

fn tab_label(session: &SessionSummary) -> String {
    let attention = if session.attention.is_empty() {
        ""
    } else {
        "!"
    };
    format!(
        "{}{}:{}",
        lifecycle_marker(session.lifecycle),
        attention,
        truncate(&session.title, 14)
    )
}

fn lifecycle_marker(lifecycle: SessionLifecycle) -> &'static str {
    match lifecycle {
        SessionLifecycle::Idle => "i",
        SessionLifecycle::Suspended => "s",
        SessionLifecycle::Working => "r",
        SessionLifecycle::WaitingForInput => "w",
        SessionLifecycle::Failed => "f",
    }
}

fn active_stats(session: &SessionSummary) -> String {
    format!(
        "duration={} tokens={} cost={}",
        format_duration(session.stats.duration),
        format_tokens(session.stats.tokens),
        format_cost(session.stats.cost_micros)
    )
}

fn format_duration(duration: std::time::Duration) -> String {
    let seconds = duration.as_secs();
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    if hours > 0 {
        format!("{hours}h{minutes:02}m")
    } else {
        format!("{minutes}m")
    }
}

fn format_tokens(tokens: u64) -> String {
    if tokens >= 1_000 {
        format!("{:.1}k", tokens as f64 / 1_000.0)
    } else {
        tokens.to_string()
    }
}

fn format_cost(cost_micros: u64) -> String {
    format!("${:.2}", cost_micros as f64 / 1_000_000.0)
}

fn truncate(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let truncated = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}

fn spans_width(spans: &[Span<'_>]) -> usize {
    spans.iter().map(|span| span.content.chars().count()).sum()
}

fn buffer_to_string(buffer: &Buffer) -> String {
    let width = buffer.area.width as usize;
    buffer
        .content()
        .chunks(width)
        .map(|row| {
            row.iter()
                .map(|cell| cell.symbol())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}
