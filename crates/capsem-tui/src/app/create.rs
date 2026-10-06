use super::*;
use capsem_sdk::models::{ImageInfo, ImageListResponse};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ImageCatalog {
    Loading,
    Loaded(Vec<ImageInfo>),
    Failed(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CreateField {
    Name,
    Image,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateDraft {
    pub name: String,
    pub field: CreateField,
    pub catalog: ImageCatalog,
    /// Zero is the ordinary VM option; subsequent positions are catalog rows.
    pub selection: usize,
    generation: u64,
}

impl CreateDraft {
    pub fn selected_image(&self) -> Option<&ImageInfo> {
        let ImageCatalog::Loaded(images) = &self.catalog else {
            return None;
        };
        self.selection.checked_sub(1).and_then(|index| images.get(index))
    }
}

impl App {
    pub(super) fn open_create(&mut self) {
        if self.pending_create.is_some() {
            self.set_control_message("a session creation is already pending");
            return;
        }
        self.pending_action = None;
        self.fork_draft = None;
        self.catalog_generation = self
            .catalog_generation
            .checked_add(1)
            .expect("catalog generation overflow");
        self.catalog_request = Some(self.catalog_generation);
        self.create_draft = Some(CreateDraft {
            name: next_session_name(&self.state),
            field: CreateField::Name,
            catalog: ImageCatalog::Loading,
            selection: 0,
            generation: self.catalog_generation,
        });
        self.overlay = AppOverlay::Create;
    }

    pub fn take_catalog_request(&mut self) -> Option<u64> {
        self.catalog_request.take().filter(|generation| {
            self.create_draft
                .as_ref()
                .is_some_and(|draft| draft.generation == *generation)
        })
    }

    pub fn retry_catalog_request(&mut self, generation: u64) {
        if self
            .create_draft
            .as_ref()
            .is_some_and(|draft| draft.generation == generation)
        {
            self.catalog_request = Some(generation);
        }
    }

    pub fn apply_image_catalog(&mut self, generation: u64, result: Result<ImageListResponse, String>) {
        if let Some(draft) = self
            .create_draft
            .as_mut()
            .or(self.pending_create.as_mut())
            .filter(|draft| draft.generation == generation)
        {
            draft.catalog = match result {
                Ok(response) => ImageCatalog::Loaded(response.images),
                Err(error) => ImageCatalog::Failed(error),
            };
            draft.selection = 0;
        }
    }

    pub fn complete_create(&mut self, success: bool) {
        let Some(draft) = self.pending_create.take() else {
            return;
        };
        if !success && self.overlay == AppOverlay::None {
            self.create_draft = Some(draft);
            self.overlay = AppOverlay::Create;
        }
    }

    pub(super) fn handle_create_key(&mut self, key: KeyEvent) -> AppAction {
        let Some(draft) = &mut self.create_draft else {
            return AppAction::Consumed;
        };
        match key.code {
            KeyCode::Esc => {
                self.create_draft = None;
                self.catalog_request = None;
                self.overlay = AppOverlay::None;
            }
            KeyCode::Tab | KeyCode::BackTab => {
                draft.field = match draft.field {
                    CreateField::Name => CreateField::Image,
                    CreateField::Image => CreateField::Name,
                };
            }
            KeyCode::Up | KeyCode::Down if draft.field == CreateField::Image => {
                if let ImageCatalog::Loaded(images) = &draft.catalog {
                    let count = images.len() + 1;
                    draft.selection = if key.code == KeyCode::Down {
                        (draft.selection + 1) % count
                    } else {
                        (draft.selection + count - 1) % count
                    };
                }
            }
            KeyCode::Enter => {
                let name = draft.name.trim().to_string();
                if name.is_empty() {
                    return AppAction::Consumed;
                }
                let image = if draft.selection == 0 {
                    None
                } else {
                    let Some(image) = draft.selected_image().filter(|image| image.image.is_some()) else {
                        self.set_control_message("this image is incompatible with the host architecture or runtime");
                        return AppAction::Consumed;
                    };
                    // Use the advertised immutable pin. The service rechecks admission.
                    image.image.clone()
                };
                let name = (name != next_session_name(&self.state)).then_some(name);
                self.pending_create = self.create_draft.take();
                self.catalog_request = None;
                self.overlay = AppOverlay::None;
                return AppAction::Invoke(ControlAction::CreateSession { name, image });
            }
            KeyCode::Backspace if draft.field == CreateField::Name => {
                draft.name.pop();
            }
            KeyCode::Char(ch)
                if draft.field == CreateField::Name
                    && !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER) =>
            {
                draft.name.push(ch);
            }
            _ => {}
        }
        AppAction::Consumed
    }
}
