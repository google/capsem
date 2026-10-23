use super::*;
use ratatui::widgets::{List, ListItem, ListState, Wrap};

pub(super) fn render(frame: &mut Frame<'_>, area: Rect, view: Option<&crate::app::ApprovalView>) {
    let Some(view) = view else { return };
    let rows = Layout::vertical([Constraint::Length(2), Constraint::Min(1), Constraint::Length(3)]).split(area);
    let message = view.error.as_deref().unwrap_or(if view.resolving {
        "Recording one decision…"
    } else if view.loading {
        "Checking current requests…"
    } else {
        "Decisions apply only to this session generation."
    });
    frame.render_widget(
        Paragraph::new(truncate(message, usize::from(area.width) * 2)).wrap(Wrap { trim: true }),
        rows[0],
    );
    let requests = view
        .list
        .as_ref()
        .map(|list| list.requests.as_slice())
        .unwrap_or_default();
    let items = requests
        .iter()
        .map(|request| {
            ListItem::new(vec![
                Line::from(Span::styled(
                    truncate(
                        &format!("{:?} · {}", request.status, request.rule_name),
                        usize::from(area.width),
                    ),
                    Style::default().fg(ACTIVE),
                )),
                Line::from(truncate(&request.summary, usize::from(area.width))),
                Line::from(Span::styled(
                    truncate(
                        &format!("{} · expires {}ms UTC", request.event_type, request.expires_at_unix_ms),
                        usize::from(area.width),
                    ),
                    muted_style(),
                )),
                Line::from(""),
            ])
        })
        .collect::<Vec<_>>();
    if items.is_empty() && !view.loading {
        frame.render_widget(Paragraph::new("No requests recorded for this session."), rows[1]);
    } else {
        let mut selection = ListState::default().with_selected(Some(view.selected));
        frame.render_stateful_widget(
            List::new(items).highlight_style(Style::default().bg(SELECTED_BG)),
            rows[1],
            &mut selection,
        );
    }
    frame.render_widget(
        Paragraph::new(vec![
            overlay_line("↑/↓ choose   A approve once   D deny"),
            overlay_line("R check current state   Esc close"),
        ]),
        rows[2],
    );
}
