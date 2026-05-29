use anyhow::Result;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Tabs, Wrap};
use ratatui::{Frame, Terminal};

use crate::model::{AppState, Attention, ServiceStatus, SessionLifecycle, SessionSummary};

pub fn render(frame: &mut Frame<'_>, state: &AppState) {
    let root = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(1),
        ])
        .split(root);

    render_light_bar(frame, state, chunks[0]);
    render_tabs(frame, state, chunks[1]);
    render_active_desktop(frame, state, chunks[2]);
    render_footer(frame, chunks[3]);
}

pub fn render_snapshot(state: &AppState, width: u16, height: u16) -> Result<String> {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend)?;
    terminal.draw(|frame| render(frame, state))?;
    Ok(buffer_to_string(terminal.backend().buffer()))
}

fn render_light_bar(frame: &mut Frame<'_>, state: &AppState, area: Rect) {
    let service = &state.service;
    let active = state.active_session();
    let service_style = match service.status {
        ServiceStatus::Online => Style::default().fg(Color::Green),
        ServiceStatus::Reconnecting | ServiceStatus::Stale | ServiceStatus::Degraded => {
            Style::default().fg(Color::Yellow)
        }
        ServiceStatus::Offline | ServiceStatus::Failed => Style::default().fg(Color::Red),
    };
    let mut spans = vec![
        Span::styled(" Capsem ", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw("svc="),
        Span::styled(service.status.label(), service_style),
        Span::raw(format!(
            " latency={}ms event-age={}ms",
            service.latency.as_millis(),
            service.last_event_age.as_millis()
        )),
    ];
    if let Some(attempt) = service.reconnect_attempt {
        spans.push(Span::raw(format!(" reconnect=#{attempt}")));
    }
    if let Some(session) = active {
        spans.push(Span::raw(" | "));
        spans.push(Span::raw(session_info(session)));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_tabs(frame: &mut Frame<'_>, state: &AppState, area: Rect) {
    let titles = state
        .sessions
        .iter()
        .map(tab_title)
        .collect::<Vec<Line<'static>>>();
    let active_index = state
        .sessions
        .iter()
        .position(|session| session.id == state.active_session_id)
        .unwrap_or_default();
    let tabs = Tabs::new(titles)
        .select(active_index)
        .block(Block::default().borders(Borders::ALL).title("desktops"))
        .style(Style::default().fg(Color::Gray))
        .highlight_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );
    frame.render_widget(tabs, area);
}

fn render_active_desktop(frame: &mut Frame<'_>, state: &AppState, area: Rect) {
    let Some(session) = state.active_session() else {
        frame.render_widget(
            Paragraph::new("No active session. Press n to create one.")
                .block(Block::default().borders(Borders::ALL).title("desktop")),
            area,
        );
        return;
    };

    let repo = session.repo_path.as_deref().unwrap_or("no repo");
    let branch = session.branch.as_deref().unwrap_or("no branch");
    let attention = attention_text(session);
    let text = vec![
        Line::from(vec![
            Span::styled(&session.title, Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(format!("  {}", session.lifecycle.label())),
        ]),
        Line::from(format!("profile: {}", session.profile)),
        Line::from(format!("repo: {repo}")),
        Line::from(format!("branch: {branch}")),
        Line::from(format!(
            "stats: jobs={} events={} cpu={} memory={}MiB",
            session.stats.jobs,
            session.stats.events,
            session.stats.cpu_percent,
            session.stats.memory_mb
        )),
        Line::from(format!("attention: {attention}")),
        Line::from(""),
        Line::from("Fixture desktop surface. Real terminal attach and HTTP state arrive in later sub-sprints."),
    ];
    let paragraph = Paragraph::new(text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("active desktop"),
        )
        .wrap(Wrap { trim: true });
    frame.render_widget(paragraph, area);
}

fn render_footer(frame: &mut Frame<'_>, area: Rect) {
    frame.render_widget(
        Paragraph::new(
            " < > switch desktop   ^ sessions   / search   : command   ? help   q quit ",
        ),
        area,
    );
}

fn tab_title(session: &SessionSummary) -> Line<'static> {
    let attention = if session.attention.is_empty() {
        ""
    } else {
        " !"
    };
    let marker = lifecycle_marker(session.lifecycle);
    Line::from(format!(" {marker} {}{attention} ", session.title))
}

fn lifecycle_marker(lifecycle: SessionLifecycle) -> &'static str {
    match lifecycle {
        SessionLifecycle::Idle => "idle",
        SessionLifecycle::Suspended => "susp",
        SessionLifecycle::Working => "work",
        SessionLifecycle::WaitingForInput => "wait",
        SessionLifecycle::Failed => "fail",
    }
}

fn session_info(session: &SessionSummary) -> String {
    let repo = session.repo_path.as_deref().unwrap_or("no repo");
    let branch = session.branch.as_deref().unwrap_or("no branch");
    format!(
        "session={} profile={} repo={} branch={}",
        session.title, session.profile, repo, branch
    )
}

fn attention_text(session: &SessionSummary) -> String {
    if session.attention.is_empty() {
        return "none".to_string();
    }
    session
        .attention
        .iter()
        .map(|attention| match attention {
            Attention::Bell => "bell",
            Attention::ApprovalRequired => "approval",
            Attention::PolicyDeny => "policy",
            Attention::CredentialIssue => "creds",
            Attention::StaleData => "stale",
        })
        .collect::<Vec<_>>()
        .join(", ")
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
