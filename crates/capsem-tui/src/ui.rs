use anyhow::Result;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Padding, Paragraph};
use ratatui::{Frame, Terminal};

use crate::app::{
    resume_blocked_reason, session_visible_in_tabs, App, AppOverlay, ControlAction, CreateDraft,
    ForkDraft,
};
use crate::model::{AppState, ServiceStatus, SessionLifecycle, SessionSummary};
use crate::model::{UpdateNotice, UpdateNoticeKind};
use crate::terminal::{TerminalColor, TerminalLine, TerminalStyle, TerminalSurface};

const MAX_VISIBLE_TABS: usize = 4;
const PREVIEW_BG: Color = Color::Rgb(17, 18, 29);
const BAR_BG: Color = Color::Rgb(24, 25, 38);
const TEXT: Color = Color::Rgb(205, 214, 244);
const MUTED: Color = Color::Rgb(127, 137, 180);
const ONLINE: Color = Color::Rgb(166, 227, 161);
const ACTIVE: Color = Color::Rgb(137, 180, 250);
const ATTENTION: Color = Color::Rgb(249, 226, 175);
const BAD: Color = Color::Rgb(243, 139, 168);
const SELECTED_BG: Color = Color::Rgb(49, 50, 68);
const LOGO_GRADIENT: [Color; 6] = [
    Color::Rgb(137, 220, 235),
    Color::Rgb(116, 199, 236),
    Color::Rgb(137, 180, 250),
    Color::Rgb(203, 166, 247),
    Color::Rgb(245, 194, 231),
    Color::Rgb(249, 226, 175),
];

pub fn render(frame: &mut Frame<'_>, state: &AppState) {
    render_with_terminal(frame, state, None);
}

pub fn render_with_terminal(
    frame: &mut Frame<'_>,
    state: &AppState,
    terminal: Option<&TerminalSurface>,
) {
    render_layout(
        frame,
        RenderLayoutCtx {
            state,
            terminal,
            overlay: AppOverlay::None,
            pending_action: None,
            control_progress: None,
            create_draft: None,
            fork_draft: None,
        },
    );
}

pub fn render_app(frame: &mut Frame<'_>, app: &App, terminal: Option<&TerminalSurface>) {
    render_layout(
        frame,
        RenderLayoutCtx {
            state: app.state(),
            terminal,
            overlay: app.overlay(),
            pending_action: app.pending_action(),
            control_progress: app.control_progress(),
            create_draft: app.create_draft(),
            fork_draft: app.fork_draft(),
        },
    );
}

struct RenderLayoutCtx<'a> {
    state: &'a AppState,
    terminal: Option<&'a TerminalSurface>,
    overlay: AppOverlay,
    pending_action: Option<&'a ControlAction>,
    control_progress: Option<&'a str>,
    create_draft: Option<&'a CreateDraft>,
    fork_draft: Option<&'a ForkDraft>,
}

fn render_layout(frame: &mut Frame<'_>, ctx: RenderLayoutCtx<'_>) {
    let root = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(root);

    if let Some(label) = ctx.control_progress {
        render_control_progress_surface(frame, chunks[0], label);
    } else {
        render_terminal_surface(frame, chunks[0], ctx.state, ctx.terminal);
    }
    render_status_bar(frame, ctx.state, chunks[1]);
    render_overlay(
        frame,
        chunks[0],
        ctx.state,
        ctx.overlay,
        ctx.pending_action,
        ctx.create_draft,
        ctx.fork_draft,
    );
}

pub fn render_snapshot(state: &AppState, width: u16, height: u16) -> Result<String> {
    Ok(buffer_to_string(&render_buffer(state, width, height)?))
}

pub fn render_svg_snapshot(state: &AppState, width: u16, height: u16) -> Result<String> {
    Ok(buffer_to_svg(&render_buffer(state, width, height)?))
}

pub fn render_app_snapshot(app: &App, width: u16, height: u16) -> Result<String> {
    Ok(buffer_to_string(&render_app_buffer(app, width, height)?))
}

pub fn render_app_svg_snapshot(app: &App, width: u16, height: u16) -> Result<String> {
    Ok(buffer_to_svg(&render_app_buffer(app, width, height)?))
}

fn render_app_buffer(app: &App, width: u16, height: u16) -> Result<Buffer> {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend)?;
    terminal.draw(|frame| render_app(frame, app, None))?;
    Ok(terminal.backend().buffer().clone())
}

fn render_buffer(state: &AppState, width: u16, height: u16) -> Result<Buffer> {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend)?;
    terminal.draw(|frame| render(frame, state))?;
    Ok(terminal.backend().buffer().clone())
}

#[cfg(test)]
pub(crate) fn render_test_buffer(state: &AppState, width: u16, height: u16) -> Result<Buffer> {
    render_buffer(state, width, height)
}

#[cfg(test)]
pub(crate) fn render_app_test_buffer(app: &App, width: u16, height: u16) -> Result<Buffer> {
    render_app_buffer(app, width, height)
}

fn render_status_bar(frame: &mut Frame<'_>, state: &AppState, area: Rect) {
    let service = &state.service;
    let active_index = state
        .sessions
        .iter()
        .position(|session| session.id == state.active_session_id)
        .unwrap_or_default();
    let base = status_base_style();
    frame.render_widget(Paragraph::new("").style(base), area);

    let mut left = vec![
        Span::styled(format!("{:>4}ms", service.latency.as_millis()), base),
        Span::styled(
            service_dot(service.status),
            service_style(service.status, service.latency.as_millis()),
        ),
        Span::styled("  ", base),
    ];
    if let Some(attempt) = service.reconnect_attempt {
        left.push(Span::styled(format!(" reconnect {attempt}"), muted_style()));
    }
    if let Some(message) = &service.control_message {
        left.push(Span::styled(
            format!(" {}", truncate(message, 28)),
            muted_style(),
        ));
    }
    if let Some(notice) = &state.update_notice {
        left.push(Span::styled(
            format!(" {}", update_notice_label(notice)),
            update_notice_style(notice),
        ));
    }

    let right = state
        .active_session()
        .map(active_stats_spans)
        .unwrap_or_else(no_session_stats_spans);

    let left_width = spans_width(&left).min(area.width as usize) as u16;
    let right_width = spans_width(&right).min(area.width as usize) as u16;
    let center_x = area.x.saturating_add(left_width);
    let reserved_width = left_width.saturating_add(right_width);
    let center_width = area.width.saturating_sub(reserved_width);
    let center = Rect::new(center_x, area.y, center_width, area.height);

    frame.render_widget(
        Paragraph::new(Line::from(left)).style(base),
        Rect::new(area.x, area.y, left_width, area.height),
    );

    if center_width > 0 {
        let tabs = tab_spans(state, active_index, center_width as usize);
        frame.render_widget(
            Paragraph::new(Line::from(tabs))
                .style(base)
                .alignment(Alignment::Center),
            center,
        );
    }

    let right_x = area
        .x
        .saturating_add(area.width.saturating_sub(right_width));
    frame.render_widget(
        Paragraph::new(Line::from(right)).style(base),
        Rect::new(right_x, area.y, right_width, area.height),
    );
}

fn update_notice_label(notice: &UpdateNotice) -> String {
    match &notice.kind {
        UpdateNoticeKind::Current => "updates: current".to_string(),
        UpdateNoticeKind::Available(tracks) => {
            let labels = tracks
                .iter()
                .map(|track| track.label())
                .collect::<Vec<_>>()
                .join(", ");
            format!("updates: {labels}")
        }
        UpdateNoticeKind::AvailableWithBlocked { available, blocked } => {
            let available = available
                .iter()
                .map(|track| track.label())
                .collect::<Vec<_>>()
                .join(", ");
            let blocked = blocked
                .iter()
                .map(|track| track.label())
                .collect::<Vec<_>>()
                .join(", ");
            format!("updates: {available}; blocked: {blocked}")
        }
        UpdateNoticeKind::Blocked(tracks) => {
            let labels = tracks
                .iter()
                .map(|track| track.label())
                .collect::<Vec<_>>()
                .join(", ");
            format!("updates blocked: {labels}")
        }
        UpdateNoticeKind::Stale => "updates: stale".to_string(),
        UpdateNoticeKind::Unavailable => "updates: unavailable".to_string(),
    }
}

fn update_notice_style(notice: &UpdateNotice) -> Style {
    match notice.kind {
        UpdateNoticeKind::Current => status_base_style().fg(MUTED),
        UpdateNoticeKind::Available(_) | UpdateNoticeKind::Stale => {
            status_base_style().fg(ATTENTION)
        }
        UpdateNoticeKind::AvailableWithBlocked { .. }
        | UpdateNoticeKind::Blocked(_)
        | UpdateNoticeKind::Unavailable => status_base_style().fg(BAD),
    }
}

fn render_control_progress_surface(frame: &mut Frame<'_>, area: Rect, label: &str) {
    let text = format!("{}...", label.trim_end_matches('.'));
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            text,
            focus_style().add_modifier(Modifier::BOLD),
        )))
        .alignment(Alignment::Center),
        area,
    );
}

fn render_terminal_surface(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AppState,
    terminal: Option<&TerminalSurface>,
) {
    if service_needs_start(state.service.status) {
        render_service_offline_surface(frame, area, state.service.status);
        return;
    }
    let Some(session) = state.active_session() else {
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled("no sessions", muted_style())),
                Line::from(Span::styled(
                    "Press Enter to create a session",
                    status_base_style().add_modifier(Modifier::BOLD),
                )),
            ])
            .alignment(Alignment::Center),
            area,
        );
        return;
    };
    if !session_accepts_terminal(session.lifecycle) {
        render_inactive_session_surface(frame, area, session);
        return;
    }

    let Some(terminal) = terminal else {
        render_waiting_terminal_surface(frame, area, session);
        return;
    };
    let active_id = session.id.as_str();
    let mut lines = terminal
        .styled_lines_for(active_id, area.height as usize)
        .into_iter()
        .map(terminal_line_to_ratatui)
        .collect::<Vec<_>>();
    if lines.is_empty() {
        let status = terminal
            .status_for(active_id)
            .unwrap_or("waiting for terminal");
        lines.push(Line::from(Span::styled(
            format!(" {status}"),
            muted_style(),
        )));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

fn render_waiting_terminal_surface(frame: &mut Frame<'_>, area: Rect, session: &SessionSummary) {
    let lines = vec![Line::from(vec![
        Span::styled("connecting terminal ", muted_style()),
        Span::styled(
            session.id.clone(),
            muted_style().add_modifier(Modifier::BOLD),
        ),
    ])];
    frame.render_widget(Paragraph::new(lines).alignment(Alignment::Center), area);
}

fn render_inactive_session_surface(frame: &mut Frame<'_>, area: Rect, session: &SessionSummary) {
    let mut lines = vec![
        Line::from(Span::styled(
            session.id.clone(),
            muted_style().add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            inactive_session_label(session.lifecycle),
            muted_style(),
        )),
    ];
    if let Some(reason) = resume_blocked_reason(session) {
        lines.push(Line::from(Span::styled(
            reason,
            bad_style().add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(Span::styled(
            "Press Enter to create a replacement",
            status_base_style().add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(Span::styled(
            "Alt+d deletes this session; Alt+p purges temporary/broken sessions",
            muted_style(),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            "Press Enter to resume",
            status_base_style().add_modifier(Modifier::BOLD),
        )));
    }
    frame.render_widget(Paragraph::new(lines).alignment(Alignment::Center), area);
}

fn render_service_offline_surface(frame: &mut Frame<'_>, area: Rect, status: ServiceStatus) {
    let lines = vec![
        Line::from(Span::styled(
            service_unavailable_title(status),
            bad_style().add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "Press Enter to start Capsem service",
            status_base_style().add_modifier(Modifier::BOLD),
        )),
    ];
    frame.render_widget(Paragraph::new(lines).alignment(Alignment::Center), area);
}

fn terminal_line_to_ratatui(line: TerminalLine) -> Line<'static> {
    let spans = line
        .spans()
        .iter()
        .map(|span| Span::styled(span.text.clone(), terminal_style_to_ratatui(span.style)))
        .collect::<Vec<_>>();
    Line::from(spans)
}

fn terminal_style_to_ratatui(style: TerminalStyle) -> Style {
    let mut result = Style::default();
    let (fg, bg) = if style.inverse {
        (style.bg, style.fg)
    } else {
        (style.fg, style.bg)
    };
    if let Some(fg) = terminal_color_to_ratatui(fg) {
        result = result.fg(fg);
    }
    if let Some(bg) = terminal_color_to_ratatui(bg) {
        result = result.bg(bg);
    }
    if style.bold {
        result = result.add_modifier(Modifier::BOLD);
    }
    if style.dim {
        result = result.add_modifier(Modifier::DIM);
    }
    if style.italic {
        result = result.add_modifier(Modifier::ITALIC);
    }
    if style.underline {
        result = result.add_modifier(Modifier::UNDERLINED);
    }
    result
}

fn session_accepts_terminal(lifecycle: SessionLifecycle) -> bool {
    matches!(
        lifecycle,
        SessionLifecycle::Working | SessionLifecycle::WaitingForInput
    )
}

fn inactive_session_label(lifecycle: SessionLifecycle) -> &'static str {
    match lifecycle {
        SessionLifecycle::Idle => "stopped",
        SessionLifecycle::Suspended => "suspended",
        SessionLifecycle::Failed => "failed",
        SessionLifecycle::Working | SessionLifecycle::WaitingForInput => "inactive",
    }
}

fn service_needs_start(status: ServiceStatus) -> bool {
    matches!(
        status,
        ServiceStatus::Offline | ServiceStatus::Degraded | ServiceStatus::Failed
    )
}

fn service_unavailable_title(status: ServiceStatus) -> &'static str {
    match status {
        ServiceStatus::Offline => "service offline",
        ServiceStatus::Degraded => "service unavailable",
        ServiceStatus::Failed => "service failed",
        ServiceStatus::Online | ServiceStatus::Reconnecting | ServiceStatus::Stale => {
            "service unavailable"
        }
    }
}

fn terminal_color_to_ratatui(color: TerminalColor) -> Option<Color> {
    match color {
        TerminalColor::Default => None,
        TerminalColor::Indexed(index) => Some(Color::Indexed(index)),
        TerminalColor::Rgb(red, green, blue) => Some(Color::Rgb(red, green, blue)),
    }
}

fn render_overlay(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AppState,
    overlay: AppOverlay,
    pending_action: Option<&ControlAction>,
    create_draft: Option<&CreateDraft>,
    fork_draft: Option<&ForkDraft>,
) {
    if overlay == AppOverlay::None {
        return;
    }
    let popup = centered_rect(area, 72, overlay_height(state, overlay));
    frame.render_widget(Clear, popup);
    let title = match overlay {
        AppOverlay::Help => " help ",
        AppOverlay::Stats => " session info ",
        AppOverlay::Home => " sessions ",
        AppOverlay::Create => " new session ",
        AppOverlay::Fork => " fork session ",
        AppOverlay::Confirm => " confirm ",
        AppOverlay::None => "",
    };
    let block = Block::new()
        .title(title)
        .borders(Borders::ALL)
        .border_style(muted_style())
        .style(status_base_style())
        .padding(Padding::horizontal(1));
    frame.render_widget(block, popup);
    let lines = match overlay {
        AppOverlay::Help => help_lines(),
        AppOverlay::Stats => stats_lines(state),
        AppOverlay::Home => home_lines(state),
        AppOverlay::Create => create_lines(state, create_draft),
        AppOverlay::Fork => fork_lines(state, fork_draft),
        AppOverlay::Confirm => confirm_lines(pending_action),
        AppOverlay::None => Vec::new(),
    };
    let inner = Rect::new(
        popup.x.saturating_add(2),
        popup.y.saturating_add(1),
        popup.width.saturating_sub(4),
        popup.height.saturating_sub(2),
    );
    frame.render_widget(Paragraph::new(lines), inner);
}

fn centered_rect(area: Rect, width_percent: u16, height: u16) -> Rect {
    let width = area.width.saturating_mul(width_percent).saturating_div(100);
    let height = height.min(area.height);
    Rect::new(
        area.x.saturating_add(area.width.saturating_sub(width) / 2),
        area.y
            .saturating_add(area.height.saturating_sub(height) / 2),
        width,
        height,
    )
}

fn overlay_height(state: &AppState, overlay: AppOverlay) -> u16 {
    match overlay {
        AppOverlay::Help => 21,
        AppOverlay::Stats => 12,
        AppOverlay::Home => (state.sessions.len() as u16).saturating_add(5).clamp(7, 16),
        AppOverlay::Create => (state.profiles.len() as u16)
            .saturating_add(10)
            .clamp(12, 18),
        AppOverlay::Fork => 8,
        AppOverlay::Confirm => 6,
        AppOverlay::None => 0,
    }
}

fn help_lines() -> Vec<Line<'static>> {
    vec![
        overlay_title("keys"),
        table_header(&["Key", "Action", "Scope", "Note"]),
        help_row("Alt+?", "help", "global", "show this table"),
        help_row("Alt+Left", "previous", "global", "switch session"),
        help_row("Alt+Right", "next", "global", "switch session"),
        help_row("Alt+1..9", "jump", "global", "select by tab number"),
        help_row("Alt+l", "sessions", "global", "list sessions and status"),
        help_row("Alt+i", "session info", "session", "active session details"),
        help_row("Alt+n", "new", "global", "create from profile"),
        help_row("Alt+f", "fork", "session", "fork active session"),
        help_row("Alt+s", "suspend", "session", "warm stop active session"),
        help_row("Alt+c", "checkpoint", "session", "save/checkpoint session"),
        help_row("Alt+r", "resume", "session", "resume inactive session"),
        help_row("Alt+t", "stop", "session", "stop active session"),
        help_row("Alt+d", "delete", "session", "delete active session"),
        help_row(
            "Alt+p",
            "purge",
            "global",
            "purge temporary/broken sessions",
        ),
        help_row("Alt+u", "update", "global", "apply binary/profile updates"),
        help_row("Alt+a", "assets", "global", "refresh VM assets"),
        help_row("Alt+q", "quit", "app", "plain q passes through"),
    ]
}

fn confirm_lines(action: Option<&ControlAction>) -> Vec<Line<'static>> {
    let Some(action) = action else {
        return vec![overlay_title("confirm"), overlay_line("no pending action")];
    };
    vec![
        overlay_title("confirm"),
        overlay_pair("action", action.label()),
        overlay_pair("target", action.target()),
        overlay_line("Enter confirms; Esc cancels"),
    ]
}

fn create_lines(state: &AppState, draft: Option<&CreateDraft>) -> Vec<Line<'static>> {
    let mut lines = vec![logo_line(), overlay_title("new session")];
    let name = draft
        .map(|draft| draft.name.as_str())
        .filter(|name| !name.is_empty())
        .unwrap_or(" ");
    lines.push(focus_pair("name", name));
    lines.push(overlay_line(
        "active input: name; type to edit; Backspace deletes",
    ));
    let create_hint = if state.profiles.is_empty() {
        "profile list unavailable; Enter disabled; Esc cancels"
    } else {
        "Up/Down selects profile; Enter creates; Esc cancels"
    };
    lines.push(overlay_line(create_hint));
    lines.push(overlay_line(""));
    lines.push(overlay_title("profiles"));
    lines.push(table_header(&["Pick", "Profile", "Name"]));

    if state.profiles.is_empty() {
        lines.push(focus_line("profiles unavailable"));
        return lines;
    }

    let selected = draft
        .map(|draft| draft.selected_profile)
        .unwrap_or_default()
        .min(state.profiles.len().saturating_sub(1));
    for (index, profile) in state.profiles.iter().take(8).enumerate() {
        let marker = if index == selected { "▶" } else { " " };
        let row = format!(
            "{marker:<4} {:<20} {:<22}",
            truncate(&profile.id, 20),
            truncate(&profile.name, 22),
        );
        if index == selected {
            lines.push(focus_line(&row));
        } else {
            lines.push(overlay_line(&row));
        }
    }
    lines
}

fn fork_lines(state: &AppState, draft: Option<&ForkDraft>) -> Vec<Line<'static>> {
    let Some(session) = state.active_session() else {
        return vec![
            overlay_title("fork session"),
            overlay_line("no active session"),
        ];
    };
    let name = draft
        .map(|draft| draft.name.as_str())
        .filter(|name| !name.is_empty())
        .unwrap_or(" ");
    vec![
        overlay_title("fork session"),
        overlay_pair("source", &session.id),
        focus_pair("name", name),
        overlay_line("active input: name; type to edit; Backspace deletes"),
        overlay_line("Enter forks; Esc cancels"),
    ]
}

fn stats_lines(state: &AppState) -> Vec<Line<'static>> {
    let Some(session) = state.active_session() else {
        return vec![
            overlay_title("session info"),
            overlay_line("no active session"),
        ];
    };
    vec![
        overlay_title("session info"),
        table_header(&["Field", "Value", "Note", ""]),
        info_row("session", &session.id, &session.title),
        info_row(
            "profile",
            &session.profile,
            session.branch.as_deref().unwrap_or(""),
        ),
        info_row(
            "state",
            session.lifecycle.label(),
            attention_summary(session),
        ),
        info_row("duration", &format_duration(session.stats.duration), ""),
        info_row("tokens", &format_tokens(session.stats.tokens), ""),
        info_row(
            "cost",
            &format!("${}", format_cost_amount(session.stats.cost_micros)),
            "",
        ),
        info_row("events", &session.stats.events.to_string(), ""),
        info_row("jobs", &session.stats.jobs.to_string(), ""),
    ]
}

fn home_lines(state: &AppState) -> Vec<Line<'static>> {
    let mut lines = vec![overlay_title("sessions")];
    if state.sessions.is_empty() {
        lines.push(overlay_line("no sessions"));
        return lines;
    }
    lines.push(table_header(&[
        "#", "Name", "Profile", "State", "Time", "Tokens", "Cost",
    ]));
    for (index, session) in state.sessions.iter().take(10).enumerate() {
        let active = if session.id == state.active_session_id {
            "▶"
        } else {
            " "
        };
        let row = format!(
            "{active} {:<2} {:<18} {:<14} {:<10} {:>6} {:>7} ${:<5}",
            index + 1,
            truncate(&session.title, 18),
            truncate(&profile_inventory_label(session), 14),
            session.lifecycle.label(),
            format_duration(session.stats.duration),
            format_tokens(session.stats.tokens),
            format_cost_amount(session.stats.cost_micros),
        );
        if session.id == state.active_session_id {
            lines.push(focus_line(&row));
        } else {
            lines.push(overlay_line(&row));
        }
    }
    lines
}

fn profile_inventory_label(session: &SessionSummary) -> String {
    if resume_blocked_reason(session).is_some() {
        return session
            .profile_status
            .clone()
            .unwrap_or_else(|| "profile-error".to_string());
    }
    session.profile.clone()
}

fn overlay_title(title: &'static str) -> Line<'static> {
    Line::from(Span::styled(
        format!(" {title}"),
        Style::default()
            .fg(ACTIVE)
            .bg(BAR_BG)
            .add_modifier(Modifier::BOLD),
    ))
}

fn logo_line() -> Line<'static> {
    let mut spans = vec![Span::styled("        ", status_base_style())];
    for (index, ch) in "CAPSEM".chars().enumerate() {
        spans.push(Span::styled(
            ch.to_string(),
            Style::default()
                .fg(LOGO_GRADIENT[index])
                .bg(BAR_BG)
                .add_modifier(Modifier::BOLD),
        ));
    }
    Line::from(spans)
}

fn overlay_line(text: &str) -> Line<'static> {
    Line::from(Span::styled(text.to_string(), status_base_style()))
}

fn focus_line(text: &str) -> Line<'static> {
    Line::from(Span::styled(text.to_string(), focus_style()))
}

fn overlay_pair(label: &'static str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:>8}  "), muted_style()),
        Span::styled(value.to_string(), status_base_style()),
    ])
}

fn focus_pair(label: &'static str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:>8}  "), muted_style()),
        Span::styled(value.to_string(), focus_style()),
    ])
}

fn table_header(columns: &[&'static str]) -> Line<'static> {
    let widths = [8, 18, 14, 12, 8, 8, 8];
    let spans = columns
        .iter()
        .enumerate()
        .map(|(index, column)| {
            Span::styled(
                format!(
                    "{column:<width$}",
                    width = widths[index.min(widths.len() - 1)]
                ),
                muted_style().add_modifier(Modifier::BOLD),
            )
        })
        .collect::<Vec<_>>();
    Line::from(spans)
}

fn help_row(
    key: &'static str,
    action: &'static str,
    scope: &'static str,
    note: &'static str,
) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{key} "),
            status_base_style().add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("{action:<14}"), status_base_style()),
        Span::styled(format!("{scope:<12}"), muted_style()),
        Span::styled(note.to_string(), status_base_style()),
    ])
}

fn info_row(field: &'static str, value: &str, note: impl AsRef<str>) -> Line<'static> {
    overlay_line(&format!("{field:<8} {value:<18} {}", note.as_ref()))
}

fn tab_spans(state: &AppState, active_index: usize, max_width: usize) -> Vec<Span<'static>> {
    let tab_sessions = state
        .sessions
        .iter()
        .enumerate()
        .filter(|(_, session)| session_visible_in_tabs(session))
        .collect::<Vec<_>>();
    if tab_sessions.is_empty() {
        return Vec::new();
    }
    let active_tab_index = tab_sessions
        .iter()
        .position(|(index, _)| *index == active_index)
        .unwrap_or_default();
    let visible = visible_tab_range(tab_sessions.len(), active_tab_index);
    let mut spans = Vec::new();
    let mut used = 0;
    if visible.start > 0 {
        push_budgeted(&mut spans, "< | ", muted_style(), max_width, &mut used);
    }
    for (offset, (session_index, session)) in tab_sessions[visible.clone()].iter().enumerate() {
        let tab_index = visible.start + offset;
        let separator = if offset == 0 && visible.start == 0 {
            ""
        } else {
            " | "
        };
        if !separator.is_empty()
            && !push_budgeted(
                &mut spans,
                separator,
                status_base_style(),
                max_width,
                &mut used,
            )
        {
            break;
        }

        if !push_tab(
            &mut spans,
            tab_index,
            session,
            *session_index == active_index,
            max_width,
            &mut used,
        ) {
            break;
        }
    }
    if visible.end < tab_sessions.len() {
        let more = " | >";
        if used + more.chars().count() <= max_width {
            spans.push(Span::styled(more, muted_style()));
        }
    }
    spans
}

fn push_tab(
    spans: &mut Vec<Span<'static>>,
    index: usize,
    session: &SessionSummary,
    active: bool,
    max_width: usize,
    used: &mut usize,
) -> bool {
    let tone = TabTone::from_session(session, active);
    let number = format!(" {} ", index + 1);
    let label = format!(
        " {}{} ",
        truncate(&session.id, 14),
        attention_marker(session)
    );
    let width = number.chars().count() + label.chars().count();
    if *used + width > max_width {
        return false;
    }

    spans.push(Span::styled(
        number,
        Style::default()
            .fg(BAR_BG)
            .bg(tone.color())
            .add_modifier(Modifier::BOLD),
    ));
    let mut label_style = Style::default().fg(tone.color()).bg(BAR_BG);
    if active {
        label_style = label_style.add_modifier(Modifier::BOLD);
    }
    if tone == TabTone::Inactive {
        label_style = label_style.add_modifier(Modifier::DIM);
    }
    spans.push(Span::styled(label, label_style));
    *used += width;
    true
}

fn push_budgeted(
    spans: &mut Vec<Span<'static>>,
    text: &str,
    style: Style,
    max_width: usize,
    used: &mut usize,
) -> bool {
    let width = text.chars().count();
    if *used + width <= max_width {
        spans.push(Span::styled(text.to_string(), style));
        *used += width;
        return true;
    }
    false
}

fn service_dot(status: ServiceStatus) -> &'static str {
    match status {
        ServiceStatus::Online => "●",
        ServiceStatus::Reconnecting | ServiceStatus::Stale | ServiceStatus::Degraded => "◐",
        ServiceStatus::Offline | ServiceStatus::Failed => "×",
    }
}

fn service_style(status: ServiceStatus, latency_ms: u128) -> Style {
    let bg = match status {
        ServiceStatus::Online if latency_ms < 100 => ONLINE,
        ServiceStatus::Online | ServiceStatus::Reconnecting | ServiceStatus::Stale => ATTENTION,
        ServiceStatus::Degraded => ATTENTION,
        ServiceStatus::Offline | ServiceStatus::Failed => BAD,
    };
    Style::default()
        .fg(bg)
        .bg(BAR_BG)
        .add_modifier(Modifier::BOLD)
}

fn status_base_style() -> Style {
    Style::default().fg(TEXT).bg(BAR_BG)
}

fn muted_style() -> Style {
    Style::default().fg(MUTED).bg(BAR_BG)
}

fn bad_style() -> Style {
    Style::default().fg(BAD).bg(BAR_BG)
}

fn focus_style() -> Style {
    Style::default()
        .fg(ATTENTION)
        .bg(SELECTED_BG)
        .add_modifier(Modifier::BOLD)
}

fn stats_style() -> Style {
    Style::default().fg(TEXT).bg(BAR_BG)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TabTone {
    Selected,
    Unselected,
    Inactive,
}

impl TabTone {
    const fn from_session(session: &SessionSummary, active: bool) -> Self {
        if matches!(
            session.lifecycle,
            SessionLifecycle::Idle | SessionLifecycle::Suspended | SessionLifecycle::Failed
        ) {
            return Self::Inactive;
        }
        if active {
            Self::Selected
        } else {
            Self::Unselected
        }
    }

    const fn color(self) -> Color {
        match self {
            Self::Selected => ATTENTION,
            Self::Unselected => ACTIVE,
            Self::Inactive => MUTED,
        }
    }
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

fn attention_marker(session: &SessionSummary) -> &'static str {
    if session.attention.is_empty() {
        ""
    } else {
        "!"
    }
}

fn attention_summary(session: &SessionSummary) -> String {
    if session.attention.is_empty() {
        return String::new();
    }
    session
        .attention
        .iter()
        .map(|attention| attention.marker())
        .collect::<Vec<_>>()
        .join(",")
}

fn active_stats_spans(session: &SessionSummary) -> Vec<Span<'static>> {
    vec![
        Span::styled(" ◷ ", muted_style()),
        Span::styled(format_duration(session.stats.duration), stats_style()),
        Span::styled(" | # ", muted_style()),
        Span::styled(format_tokens(session.stats.tokens), stats_style()),
        Span::styled(" | $ ", muted_style()),
        Span::styled(format_cost_amount(session.stats.cost_micros), stats_style()),
        Span::styled(" | help: alt+?", muted_style()),
        Span::styled(" ", stats_style()),
    ]
}

fn no_session_stats_spans() -> Vec<Span<'static>> {
    vec![
        Span::styled(" no session", muted_style()),
        Span::styled(" | help: alt+?", muted_style()),
        Span::styled(" ", stats_style()),
    ]
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

fn format_cost_amount(cost_micros: u64) -> String {
    format!("{:.2}", cost_micros as f64 / 1_000_000.0)
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

fn buffer_to_svg(buffer: &Buffer) -> String {
    const CHAR_WIDTH: usize = 11;
    const LINE_HEIGHT: usize = 22;
    const FONT_SIZE: usize = 16;
    const PAD: usize = 16;

    let width = buffer.area.width as usize;
    let height = buffer.area.height as usize;
    let svg_width = width * CHAR_WIDTH + PAD * 2;
    let content_height = height * LINE_HEIGHT + PAD * 2;
    let svg_height = svg_width.max(content_height);
    let mut svg = String::new();
    svg.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{svg_width}\" height=\"{svg_height}\" viewBox=\"0 0 {svg_width} {svg_height}\">\n"
    ));
    svg.push_str(&format!(
        "<rect width=\"100%\" height=\"100%\" fill=\"{}\"/>\n",
        color_hex(PREVIEW_BG)
    ));
    svg.push_str("<style>text{font-family:Menlo,Monaco,Consolas,monospace;dominant-baseline:text-before-edge;}</style>\n");

    for y in 0..height {
        for x in 0..width {
            let cell = &buffer.content()[y * width + x];
            let bg = if cell.bg == Color::Reset {
                PREVIEW_BG
            } else {
                cell.bg
            };
            let rect_x = PAD + x * CHAR_WIDTH;
            let rect_y = PAD + y * LINE_HEIGHT;
            svg.push_str(&format!(
                "<rect x=\"{rect_x}\" y=\"{rect_y}\" width=\"{CHAR_WIDTH}\" height=\"{LINE_HEIGHT}\" fill=\"{}\"/>\n",
                color_hex(bg)
            ));

            let symbol = cell.symbol();
            if symbol == " " {
                continue;
            }
            let fg = if cell.fg == Color::Reset {
                TEXT
            } else {
                cell.fg
            };
            let weight = if cell.modifier.contains(Modifier::BOLD) {
                "700"
            } else {
                "400"
            };
            svg.push_str(&format!(
                "<text x=\"{rect_x}\" y=\"{rect_y}\" font-size=\"{FONT_SIZE}\" font-weight=\"{weight}\" fill=\"{}\">{}</text>\n",
                color_hex(fg),
                escape_xml(symbol)
            ));
        }
    }
    svg.push_str("</svg>\n");
    svg
}

fn color_hex(color: Color) -> String {
    match color {
        Color::Reset => color_hex(TEXT),
        Color::Black => "#000000".to_string(),
        Color::Red => "#f38ba8".to_string(),
        Color::Green => "#a6e3a1".to_string(),
        Color::Yellow => "#f9e2af".to_string(),
        Color::Blue => "#89b4fa".to_string(),
        Color::Magenta => "#cba6f7".to_string(),
        Color::Cyan => "#89dceb".to_string(),
        Color::Gray => "#bac2de".to_string(),
        Color::DarkGray => "#585b70".to_string(),
        Color::LightRed => "#f38ba8".to_string(),
        Color::LightGreen => "#a6e3a1".to_string(),
        Color::LightYellow => "#f9e2af".to_string(),
        Color::LightBlue => "#89b4fa".to_string(),
        Color::LightMagenta => "#cba6f7".to_string(),
        Color::LightCyan => "#89dceb".to_string(),
        Color::White => "#ffffff".to_string(),
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        Color::Indexed(index) => {
            let gray = index.max(16);
            format!("#{gray:02x}{gray:02x}{gray:02x}")
        }
    }
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
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
