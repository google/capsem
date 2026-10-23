use super::*;

pub(super) fn home_lines(state: &AppState) -> Vec<Line<'static>> {
    let mut lines = vec![
        overlay_title("agent manager"),
        overlay_line("New session [N]"),
        overlay_line("Up/Down chooses a session; Enter selects; Esc closes"),
    ];
    if state.sessions.is_empty() {
        lines.push(overlay_line("no sessions"));
        return lines;
    }
    lines.push(table_header(&["#", "Name", "State", "Latest terminal activity"]));
    for (index, session) in state.sessions.iter().take(10).enumerate() {
        let active = if session.id == state.active_session_id {
            "▶"
        } else {
            " "
        };
        let activity = session.terminal.as_ref().map(|terminal| {
            terminal
                .last_notification
                .as_ref()
                .map_or(terminal.title.as_str(), |notification| notification.title.as_str())
        });
        let row = format!(
            "{active} {:<2} {:<18} {:<10} {}",
            index + 1,
            truncate(&session.title, 18),
            session.lifecycle.label(),
            truncate(
                &format!(
                    "{}{}",
                    if session.attention.contains(&crate::model::Attention::ApprovalRequired) {
                        "approval · "
                    } else {
                        ""
                    },
                    activity.unwrap_or("unavailable")
                ),
                28
            ),
        );
        if session.id == state.active_session_id {
            lines.push(focus_line(&row));
        } else {
            lines.push(overlay_line(&row));
        }
    }
    if let Some(session) = state.active_session() {
        if let Some(reason) = resume_blocked_reason(session) {
            lines.push(overlay_line(reason));
        }
        let mut actions = vec!["Enter: Select"];
        for (action, label) in [
            (capsem_sdk::models::VmAction::Stop, "Alt+T: Stop"),
            (capsem_sdk::models::VmAction::Fork, "Alt+F: Fork"),
            (capsem_sdk::models::VmAction::Delete, "Alt+D: Delete"),
            (capsem_sdk::models::VmAction::Start, "Alt+R: Start"),
            (capsem_sdk::models::VmAction::Resume, "Alt+R: Resume"),
        ] {
            if session.available_actions.contains(&action) {
                actions.push(label);
            }
        }
        lines.push(overlay_line(&actions.join("  ")));
        if let Some(terminal) = &session.terminal {
            lines.push(overlay_pair("terminal", &truncate(&terminal.title, 80)));
            if let Some(notification) = &terminal.last_notification {
                lines.push(overlay_pair(
                    "notification",
                    &truncate(&format!("{} {}", notification.title, notification.body), 100),
                ));
            }
            if terminal.bell_count > 0 {
                lines.push(overlay_pair("bells", &terminal.bell_count.to_string()));
            }
        } else {
            lines.push(overlay_line("Terminal activity unavailable"));
        }
    }
    lines
}
