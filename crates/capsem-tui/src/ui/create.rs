use super::*;
use crate::app::{CreateField, ImageCatalog};

pub(super) fn create_lines(draft: Option<&CreateDraft>) -> Vec<Line<'static>> {
    let mut lines = vec![logo_line(), overlay_title("new session")];
    let Some(draft) = draft else { return lines };
    let pair = |field, label, value: &str| {
        if draft.field == field {
            focus_pair(label, value)
        } else {
            overlay_pair(label, value)
        }
    };
    lines.push(pair(
        CreateField::Name,
        "name",
        if draft.name.is_empty() { " " } else { &draft.name },
    ));
    lines.push(pair(
        CreateField::Image,
        "image",
        draft.selected_image().map_or("Plain VM", |image| image.name.as_str()),
    ));
    match &draft.catalog {
        ImageCatalog::Loading => lines.push(overlay_line("loading image catalog")),
        ImageCatalog::Failed(error) => lines.push(overlay_line(&truncate(error, 160))),
        ImageCatalog::Loaded(images) if images.is_empty() => lines.push(overlay_line("no catalog images available")),
        ImageCatalog::Loaded(_) => {
            if let Some(image) = draft.selected_image() {
                lines.push(overlay_line(&truncate(&image.description, 120)));
                lines.push(overlay_pair(
                    "compatibility",
                    if image.image.is_some() {
                        "compatible"
                    } else {
                        "incompatible architecture/runtime"
                    },
                ));
                lines.push(overlay_pair(
                    "cache",
                    match image.cached {
                        capsem_sdk::models::ImageCacheState::Unknown => "unknown",
                    },
                ));
                if let Some(pin) = &image.image {
                    lines.push(overlay_pair("image pin", &truncate(pin, 120)));
                }
            } else {
                lines.push(overlay_line("Plain VM uses the service's default environment"));
            }
        }
    }
    lines.push(overlay_line("Tab changes field; Up/Down chooses an image"));
    lines.push(overlay_line("active input: name; type to edit; Backspace deletes"));
    lines.push(overlay_line("Enter creates; Esc cancels"));
    lines
}
