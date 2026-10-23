use super::*;
use crate::app::{CreateField, ImageCatalog};
use capsem_sdk::models::{ImageCacheState, ImageInfo};
use ratatui::widgets::{Row, Table, TableState};

pub(super) fn render(frame: &mut Frame<'_>, area: Rect, draft: Option<&CreateDraft>, message: Option<&str>) {
    let Some(draft) = draft else { return };
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(2),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(3),
    ])
    .split(area);
    let name = if draft.field == CreateField::Name {
        focus_pair("Name", &draft.name)
    } else {
        overlay_pair("Name", &draft.name)
    };
    frame.render_widget(Paragraph::new(name), rows[0]);
    frame.render_widget(Paragraph::new(overlay_title("Image list")), rows[2]);
    match &draft.catalog {
        ImageCatalog::Loading => {
            frame.render_widget(Paragraph::new("Loading image catalog…").style(muted_style()), rows[3])
        }
        ImageCatalog::Failed(error) => {
            frame.render_widget(Paragraph::new(truncate(error, 512)).style(bad_style()), rows[3])
        }
        ImageCatalog::Loaded(images) if images.is_empty() => frame.render_widget(
            Paragraph::new("No terminal apps available").style(muted_style()),
            rows[3],
        ),
        ImageCatalog::Loaded(images) => {
            let items = images.iter().map(|image| {
                Row::new(vec![
                    truncate(
                        image
                            .app
                            .as_ref()
                            .and_then(|app| app.publisher.as_deref())
                            .unwrap_or("Not provided"),
                        80,
                    ),
                    truncate(&image.name, 80),
                    status(image).into(),
                    truncate(&image.description, 512),
                ])
                .style(if image.image.is_some() {
                    status_base_style()
                } else {
                    muted_style()
                })
            });
            let table = Table::new(
                items,
                [
                    Constraint::Percentage(16),
                    Constraint::Percentage(24),
                    Constraint::Percentage(22),
                    Constraint::Percentage(38),
                ],
            )
            .header(
                Row::new(["Owner", "Image", "Status", "Description"]).style(muted_style().add_modifier(Modifier::BOLD)),
            )
            .row_highlight_style(Style::default().fg(ACTIVE).bg(SELECTED_BG).add_modifier(Modifier::BOLD))
            .highlight_symbol("> ");
            let selected = draft
                .selected_image()
                .and_then(|selected| images.iter().position(|image| image.name == selected.name));
            frame.render_stateful_widget(table, rows[3], &mut TableState::default().with_selected(selected));
        }
    }
    let detail = message.map(str::to_owned).unwrap_or_else(|| {
        if draft.caching {
            "Preparing image…".into()
        } else if let Some(image) = draft.selected_image() {
            format!("{} · {}", image.name, status(image))
        } else {
            "Choose a compatible terminal app".into()
        }
    });
    frame.render_widget(Paragraph::new(truncate(&detail, 512)).style(muted_style()), rows[4]);
    let legend = vec![
        overlay_line("↑/↓ choose image · Tab name/image · Esc cancel"),
        overlay_line("Enter creates · missing images prepare automatically"),
        overlay_line("Ctrl-P prepare only · Ctrl-R retry catalog"),
    ];
    frame.render_widget(Paragraph::new(legend), rows[6]);
}

fn status(image: &ImageInfo) -> &'static str {
    if image.image.is_none() {
        return "Unavailable";
    }
    if image.progress.is_some() {
        return "Preparing";
    }
    if image.cached == ImageCacheState::Ready {
        return "Cached";
    }
    if image.previous_image.is_some() {
        return "Update available";
    }
    match image.cached {
        ImageCacheState::Unknown => "Checking",
        ImageCacheState::Missing | ImageCacheState::Partial => "Needs download",
        ImageCacheState::Ready => "Cached",
    }
}
